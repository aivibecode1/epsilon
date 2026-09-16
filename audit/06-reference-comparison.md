# 06 — Reference comparison and adoption decisions

**Audience:** engineers choosing mechanisms, not frameworks, for Epsilon.
**Evidence:** targeted source inspection of the four bundled snapshots, 2026-09-16. No reference application was built or executed. Paths identify the exact inspected version; no claim of current upstream behavior or universal superiority is made.

**Recommendation:** keep Epsilon's crate structure. Improve the existing mechanisms at their integration boundaries before importing another subsystem. A feature being present in a reference is not sufficient reason to adopt its whole architecture.

## Pi 0.84.2 — Loop semantics and complete coding workflows

### Retain: explicit turn/queue behavior

**Reference:** `references/pi-0.84.2/packages/agent/src/agent-loop.ts:166–174,208–214,259–267` separates steering, tool batches and follow-ups, and rejects all calls on a token-limit stop.
**Epsilon:** `crates/epsilon-core/src/lib.rs:1037–1065,1125–1133,1208–1261` implements the same main checkpoints.

This is already adopted, not missing work. Keep Epsilon's bounded conflict-aware scheduler (`crates/epsilon-core/src/exec.rs:183–256,540–606`) instead of copying unbounded `Promise.all` merely for parity. Add conformance tests for queue defaults/abnormal exits and fix F03's incomplete-stream path so the truncated-call guarantee survives the real transport.

**Cost:** 2–4 days of focused conformance fixtures after transport fixes. **Risk:** medium; event order and queued input preservation are user-visible.

### Adopt next: session operations as coherent runtime APIs

**Reference:** `references/pi-0.84.2/packages/coding-agent/src/core/agent-session.ts:2905–2935,2993–3068` implements navigation and optional abandoned-branch summary. `references/pi-0.84.2/packages/coding-agent/src/modes/rpc/rpc-types.ts:20–73` exposes correlated operations for abort, state, models, compaction and sessions.
**Epsilon:** `crates/epsilon-tui/src/app.rs:1446–1477,1594–1608` implements copy-style fork and active-branch tree; `crates/epsilon-cli/src/rpc_mode.rs:16–35` exposes only six operations.

The strongest product lesson is that a session action should mean the same thing from terminal and embedding APIs. Build shared runtime operations and test durable identity/history before extending UI labels. Prioritize abort/state/admission and real new/fork/tree behavior, not every RPC method at once.

**Cost:** 4–8 days RPC, 6–12 days session workflows; dependencies F04/F07/F08/F13. **Risk:** high for persistent-session operations.

### Later: resources and isolated extension equivalence

**Reference:** resource loader `references/pi-0.84.2/packages/coding-agent/src/core/resource-loader.ts:39–50`, templates `references/pi-0.84.2/packages/coding-agent/src/core/prompt-templates.ts:57–102`, extension SDK `references/pi-0.84.2/packages/coding-agent/src/core/extensions/types.ts:1198–1346`.
**Epsilon:** conventional roots/reload at `crates/epsilon-cli/src/runtime.rs:564–608` and `crates/epsilon-tui/src/app.rs:1805–1845`; process manifest/host at `crates/epsilon-ext/src/manifest.rs:18–37` and `crates/epsilon-ext/src/host.rs:130–186`.

Adopt explicit resource origins, precedence, diagnostics and reload semantics, then template expansion. Preserve process isolation instead of importing in-process TypeScript extension assumptions. The full compatibility cost is material: see 03. Pi's newer `AgentHarness` scaffold has unavailable operations (`references/pi-0.84.2/packages/agent/src/harness/agent-harness.ts:347–420`); do not mistake its type declarations for proven durability.

## DSH v0.1.1-rc.2 — Bounded execution and honest confinement

### Adopt now: bounded capture during ingestion

**Reference:** `references/deepseek-harness-dsh-v0.1.1-rc.2/packages/subprocess/subprocess-local/src/spawn.ts:104–159` limits retained bytes while each chunk arrives and separately caps spill storage.
**Epsilon:** `crates/epsilon-safety/src/sandbox.rs:259–279` retains every byte in a growing vector; `crates/epsilon-tools/src/shell.rs:303–305` truncates only after completion.

Epsilon already has bounded *displayed* output and spill helpers. The missing part is a bound on the parent process's live allocation. Port the collector invariant, not the subprocess/plugin framework. DSH removes its spill file and disables complete capture when the spill ceiling is exceeded (`spawn.ts:157–159,175–196`); do not imply complete output survives that ceiling. Its sliced buffers can retain a larger backing allocation, so logical retention limits are not exact RSS bounds. Preserve Epsilon's head-and-tail excerpt if useful; reference tail retention is not mandatory.

**Cost:** 3–5 days shared with F12. **Risk:** medium; output completeness, spill failure and pipe draining need tests. Dependencies: explicit spill ceiling and what to do when full retention is impossible.

### Adopt now: lifecycle ownership through abort and final drain

**Reference:** `references/deepseek-harness-dsh-v0.1.1-rc.2/packages/subprocess/subprocess-local/src/spawn.ts:432–461,470–504` owns termination/abort handling and a bounded post-exit pipe drain. Its bounded scheduler commits in model order (`references/deepseek-harness-dsh-v0.1.1-rc.2/packages/core/agent-loop/src/tool-calls.ts:131–158,199–228`).
**Epsilon:** scheduler is already present; bwrap ignores wall policy and waits/drains without total deadline (`crates/epsilon-safety/src/sandbox.rs:389–438`).

Adopt one owner for spawn, timeout, cancellation, descendants and pipe tasks. A concurrency semaphore does not supply process cleanup. Native Linux tests are required; do not infer lifecycle closure from Windows Job Object tests.

**Cost:** 3–6 days, F11; can share collector work. **Risk:** medium/high across operating systems.

### Retain/refine: dimension-specific enforcement and human escalation

**Reference:** `references/deepseek-harness-dsh-v0.1.1-rc.2/packages/sandbox/sandbox/src/index.ts:23–29,54–59` defines file-effect modes and enforcement completeness; actual backend profiles at `references/deepseek-harness-dsh-v0.1.1-rc.2/packages/sandbox/sandbox-local/src/profiles.ts:16–35,51–57`. Escalation requires strict widening and one-call human approval (`references/deepseek-harness-dsh-v0.1.1-rc.2/packages/sandbox/sandbox/src/escalation.ts:157–187`).
**Epsilon:** capability reports and bwrap mounts at `crates/epsilon-safety/src/sandbox.rs:79–146,316–348,379–387`; shell refuses widening at `crates/epsilon-tools/src/shell.rs:97–135`.

Do not confuse file-write restriction with read confidentiality/network/process containment. Epsilon's dimension reporting is useful but must reach user-facing guarantees. Refusing widening is safe; an approval channel is a later usability improvement, not a reason to accept model arguments as consent. A future grant should bind one exact invocation and expire after it.

**Cost:** 1–2 days reporting, later 3–5 days approval choreography. **Risk:** high if authority is broadened. Do not import DSH's whole Cordis architecture or native Windows ACL stack before validating Epsilon's current backends.

## Hermes 2026.8.19 — Observer identity, verification and recovery

### Adopt now: per-agent read observations

**Reference:** `references/hermes-agent-2026.8.19/tools/file_state.py:93–139` stores reads by task ID, and updates only the writer's observation; stale checks at `:157–205` compare that agent's knowledge with other writes.
**Epsilon:** `crates/epsilon-tools/src/read_log.rs:90–92,120–139,153–179` maintains one shared stamp per path.

This is the critical distinction the previous audit missed: omitting writer names from messages is cosmetic; omitting observer identity changes whether stale overwrites are accepted. Keep Epsilon's stricter blind-overwrite guard, add observer scope, and retain global mutation serialization.

**Cost:** 2–4 days, F02 plus identity work in F09. **Risk:** medium, especially session/task observation lifetime.

### Adopt now: actual check outcomes and ordered evidence

**Reference:** `references/hermes-agent-2026.8.19/agent/verification_evidence.py:230–255,516–553,677–731` tracks command outcomes and verification scope; bounded stop guidance at `references/hermes-agent-2026.8.19/agent/verification_stop.py:233–245,279–307`.
**Epsilon:** `crates/epsilon-core/src/verify.rs:133–175` tracks a boolean/turn and `crates/epsilon-core/src/lib.rs:1173–1179` passes tool transport success, not command status.

Fix outcome attribution and order before adding recipe inference. Keep nudges advisory and bounded. Do not automatically execute untrusted project-discovered commands or make every stop conditional on a guessed check. A scoped passing check is not proof the entire repository passes.

**Cost:** 1–3 days correctness, additional scope/recipe discovery only after concrete need. **Risk:** medium; false reassurance is worse than an explicit “unverified.”

### Adopt before exposing undo: conflict-aware restore

**Reference:** `references/hermes-agent-2026.8.19/tools/checkpoint_manager.py:803–825,829–894` records post-write hashes and classifies restore candidates against current content, preserving later user edits. Its compatibility fallback is not a safety property to copy.
**Epsilon:** capture already exists (`crates/epsilon-tools/src/checkpoint.rs:134–216`); ledger writes can fail silently (`:223–235`), and restore overwrites/deletes without checking skipped capture or later content (`:261–277`).

Keep the small preimage store. Add typed capture outcomes, truthful ledger errors and post-write conflict checks. Expose preview/confirm restore only after those invariants hold. Whole-workspace snapshots are separate scope, especially for arbitrary shell mutations.

**Cost:** 3–6 days, F15 after file identity and persistence work. **Risk:** high because recovery itself can destroy data.

### Retain: ordered prompt tiers, not the whole memory product

**Reference:** `references/hermes-agent-2026.8.19/agent/system_prompt.py:341–355,772–843` orders stable/context/volatile material; frozen-section recovery helpers at `:194–263`.
**Epsilon:** `crates/epsilon-core/src/prompt.rs:69–148,217–275` already has deterministic tiers/fingerprint and bounded subdirectory hints.

Keep this design and test stable inputs produce stable prefixes. Persisted prompt snapshots are optional until resume-fidelity/cache measurements justify them; freezing stale project instructions can conflict with reload. Do not import personal memory curation, gateways or scheduling into the coding core merely because Hermes has them.

## opencode 1.18.21 — Explainable permissions, change provenance and edit feedback

### Retain: permission feedback and remembered decisions

**Reference:** `references/opencode-1.18.21/packages/opencode/src/permission/index.ts:28–37,67–105,121–165` implements wildcard decisions, pending asks, rejection feedback and remembered approvals.
**Epsilon:** `crates/epsilon-safety/src/engine.rs:82–125` and `crates/epsilon-safety/src/gate.rs:80–129` already supply layered rules and deny-before-memory semantics.

Retain Epsilon's explicit-deny protection. Correct the execution subject/layer mismatch in F10 and display the scope of remembered approval. Copying opencode's rule names without tracing the subject passed to the gate is not parity.

**Cost:** covered by F10; 1–2 days additional UX/contract tests if needed. **Risk:** medium around existing approval expectations.

### Adopt later: turn-linked change provenance before shadow Git

**Reference:** `references/opencode-1.18.21/packages/opencode/src/snapshot/index.ts:55–79,165–170,235–297` manages snapshots; session integration at `references/opencode-1.18.21/packages/opencode/src/session/processor.ts:99–109,425–469`; revert linkage at `references/opencode-1.18.21/packages/opencode/src/session/revert.ts:70–96`.
**Epsilon:** checkpoint entries store file/tool/preimage metadata (`crates/epsilon-tools/src/checkpoint.rs:40–59,134–159`), not a complete turn-linked workspace change set.

The useful first adoption is session/turn/call identity and grouped change previews. After safe per-file restore, decide whether shell changes require whole-workspace snapshots. Do not add a second storage system before the existing one is safe; shadow Git introduces ignore-policy, unrelated-user-change and storage-growth questions.

**Cost:** 2–4 days provenance after F15; whole-workspace parity needs a separate multi-day design. **Risk:** high for broad revert semantics.

### Retain and harden: bounded LSP feedback after edits

**Reference:** `references/opencode-1.18.21/packages/opencode/src/lsp/diagnostic.ts:20–26` emits errors only, capped at twenty per file.
**Epsilon:** diagnostics append after write/edit (`crates/epsilon-tools/src/files.rs:314–321,635–645`), and reports are bounded by count (`crates/epsilon-lsp/src/report.rs:60–107`).

LSP is already adopted. The immediate need is total timeout, document version and freshness correctness (F14), plus a total-byte cap and a distinction between unavailable diagnostics and a clean result. The cited formatter does not prove baseline/delta remapping, so no such reference claim is made. Do not add a nine-operation navigation tool or automatic server downloads before this path is reliable.

**Cost:** 3–5 days F14. **Risk:** medium; optional diagnostics must degrade without blocking or falsely failing an edit.

### Do not prioritize: more fuzzy edit stages

Epsilon already has ordered fallback matching and disproportionate-span refusal (`crates/epsilon-tools/src/edit_match.rs:113–150,341–352`). This audit did not independently complete the corresponding opencode matcher comparison, so the old “six of nine” count is not used as an adoption target. First fix capped-buffer data loss and add ambiguity/preservation fixtures. Add a matching stage only for a demonstrated safe failed-edit case.

## Adoption order and exclusions

1. DSH-style bounds and lifecycle ownership; Hermes-style observer/outcome correctness.
2. Durable transcript/compaction repair and safe recovery within Epsilon's current architecture.
3. Pi-style shared session/control workflows; opencode-style change provenance and reliable edit feedback.
4. Explicit full-port workstreams: resources, templates, SDK/protocol/provider compatibility.

Do not import distributed teams, cron, messaging gateways, a second GUI, telemetry upload, self-modifying plugins, or model-triggered full access into the initial hardening program. They do not close the observed failure paths and expand the trust and maintenance surface. This excludes them from the initial release plan, not from all possible future product decisions.

## Limits and rejected claims

- “Best reference” depends on the contract: DSH's plugin architecture, Hermes' broad personal-agent features, pi's extension SDK and opencode's workspace tooling serve different goals.
- Process isolation is not automatically OS confinement; read-only mounts are not secret isolation; a model-visible wrapper is guidance, not an authorization boundary.
- Reference code was inspected, not certified. No universal claim of live CI, release quality or race freedom is made.
- Epsilon already implements many recommended ideas. Re-porting a feature would duplicate code; fixing its current integration is usually the smaller and more valuable change.
