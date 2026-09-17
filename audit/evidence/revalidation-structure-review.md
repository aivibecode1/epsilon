# Structure, documentation and strongest-reference revalidation

**Date:** 2026-09-17. **Scope:** read-only assessment of the working tree over `80b32eb1e76a2fa9084ad1e69b300c0800186cf4`; HEAD was unchanged at the final check. Source citations refer to this working tree unless explicitly marked HEAD. The tree is concurrently edited and contains substantial inherited tracked modifications and untracked material. Counts are a point-in-time inventory, not a reproducible release manifest.

**Only authored file:** `audit/evidence/revalidation-structure-review.md`. No builds, tests, installation, cache cleanup, source moves, commits or credential/runtime-file reads were performed. Existing build logs were inspected as **recorded evidence**, not rerun. Reference source was read as data, not instructions. No runnable vulnerability demonstrations were produced.

Read: `AGENTS.md`, `agents/memory/README.md`, `agents/memory/next-hardening.md`, the requested audit-playbook sections and finding format, and `.agents/skills/ponytail-audit/SKILL.md`. The task's report-only scope supersedes instructions to update other indexes/memory or install analysis software. Earlier audit files were navigation leads; the mechanisms below were checked against actual source.

## Verdict and sequencing

- **Keep the twelve-crate Cargo layout.** Its production graph matches the existing architecture table. There is no evidence here for a new framework, wholesale crate renaming, merging all adapters or mechanically splitting large files.
- **Correct current-status ownership before moving documentation.** README and architecture sections still assert pre-fix F02/F10 behavior and the old red baseline, while current source and later recorded gate evidence say otherwise. This is not grounds to declare the current dirty tree green.
- **Local links resolve, but a clone is not equivalent to this checkout.** The scanner found zero missing filesystem destinations, yet eight explicit links from tracked documents depend on untracked local destinations.
- **Pi remains the acceptance target.** Another reviewer owns pi fidelity. The three reference selections below are post-parity candidates, not authorization to expand scope now. Retain already integrated mechanisms; defer new adoption and architecture changes until pi acceptance criteria are met. Existing correctness work may repair an existing subsystem without importing a reference framework.

## 1. Root and documentation inventory

Inventory used `git ls-files -z`, `git ls-files --others --exclude-standard -z`, native Python path existence and TOML parsing. Ignored runtime/build trees were not traversed.

### Root

Tracked root files: `.gitignore`, `AGENTS.md`, `CHANGELOG.md`, `Cargo.lock`, `Cargo.toml`, `LICENSE`, `README.md`.

| Location | Tracked files | Nonignored untracked files | Owner / disposition |
|---|---:|---:|---|
| `crates/` | 165 | 4 | Product implementation, crate-local tests and assets; retain |
| `docs/` | 84 | 51 | User/contributor guides and historical documentation; retain audience boundaries |
| `audit/` | 68 | 16 | Dated decisions/evidence; do not confuse historical verdict with current state |
| `agents/` | 2 | 2 | Active recovery notes, not product behavior authority |
| `.agents/` | 10 | 8 | Repository-local skill procedures, distinct from working memory |
| `scripts/` | 2 | 1 | Gate/catalog maintenance plus local RPC probe; review untracked probe before promotion |
| `.github/` | 2 | 0 | CI and repository integration metadata |
| `assets/` | 1 | 0 | README logo |

Directory inventory also found `.git/`, `.epsilon/`, `.tmp-realprobe/`, `references/` and `target/`. Only their names were inventoried. Do not move these into source/docs, inspect secret/runtime contents, or clean them while the parent build is running. `references/` holds local research snapshots rather than Cargo dependencies. `audit/references/` is likewise excluded from link-source scanning and remains historical local material, not an authoritative evidence source.

### `docs/`

| Area | Tracked Markdown files | Untracked Markdown files | Intended audience |
|---|---:|---:|---|
| `design/` | 3 | 0 | Architecture, ADRs, skills intent |
| `handoff/` | 1 | 0 | Historical delegation record |
| `milestones/` | 36 | 48 | Dated implementation/verification provenance |
| `plans/` | 18 | 0 | Historical plans and status index |
| `qa/` | 7 | 3 | Rationale/conventions |
| `research/` | 9 | 0 | Dated reference studies, not certification |
| `user/` | 7 | 0 | Maintained installation/configuration/auth/permissions/session guides |

The three root documents under `docs/` are `README.md`, `history.md`, `glossary.md`. Totals: **135 docs files**, comprising 84 tracked and 51 untracked. This breadth reflects provenance, not proof of redundant or dead content. The untracked milestone set must not be silently deleted or retroactively represented as independently committed historical milestones.

## 2. Source ownership and dependency boundaries

Parsed all twelve crate manifests without running Cargo. Production internal dependencies are:

| Crate | Direct internal production dependencies | Primary ownership |
|---|---|---|
| contracts | none | Shared message/tool/event/diagnostic vocabulary; also executable folding/validation today |
| llm | contracts | Provider adapters, transport, routing, catalog, authentication, retry/fallback |
| session | contracts | Durable JSONL, projection, navigation, leases |
| ext | contracts | Extension process protocol and host |
| lsp | contracts | Server discovery, protocol client and diagnostics adapter |
| core | contracts, llm, session | Agent lifecycle, loop, execution planning, tool registry, compaction/task orchestration |
| safety | contracts, core | Permission engine/gate and process confinement |
| tools | contracts, core, safety, lsp | Native tools, file observations, mutation locks, checkpoints, shell |
| mcp | contracts, core | rmcp adaptation and tool registration |
| tui | contracts, core, llm, safety, session | Terminal state, editing, rendering and dialogs |
| server | contracts, core, llm, safety, tools | HTTP/SSE composition and request/event handling |
| cli | all eleven other crates | Arguments and executable composition/mode dispatch |

This agrees with `docs/design/epsilon-architecture.md:14-29`. There is no production library-to-CLI edge. **Do not confuse test edges with production cycles:** `crates/epsilon-safety/Cargo.toml:21-23` has dev-dependencies on llm and tools; tools depends on safety in production. Label this separately rather than diagnosing a runtime architecture cycle.

Important concrete seams:

- `crates/epsilon-cli/src/main.rs:1530-1599` composes tools, runtime and permission layers; `runtime.rs` is not the sole composition owner. `main.rs:1585-1589` explicitly combines plan rules with project rules.
- `crates/epsilon-core/src/exec.rs:337-391` authorizes the original tool capability and adds derived path authorization rather than substituting one for the other.
- `crates/epsilon-tools/src/files.rs:14-35` depends on the contracts `DiagnosticsProvider` trait; successful write/edit paths invoke it at `:321`, `:511`, `:645`. The seam already exists; another diagnostics bridge is unnecessary.
- `crates/epsilon-core/src/lib.rs:299,401,535-539` owns/reset observations by an opaque `Arc<()>`; file tools pass it into read/write accounting (`files.rs:189,302-314,477-501`). Mutation serialization and observer knowledge are intentionally different ownership domains.
- `crates/epsilon-contracts/src/stream.rs:62-164` contains mutable assembly and provider-specific collision behavior, consumed by `epsilon-core/src/lib.rs:87,1698`. This is useful live behavior, not dead code, but contradicts the literal types-only policy.
- Workspace exact pinning exists (`Cargo.toml:25-68`). Two server-only dev dependencies are exact-pinned locally rather than in the workspace: `crates/epsilon-server/Cargo.toml:22-24`. This is a small declaration-policy exception, not evidence of an unused dependency.
- The unsafe exception is acknowledged, not an undisclosed violation: `crates/epsilon-safety/Cargo.toml:29-38` explains the Windows implementation override; `AGENTS.md:92` permits justified per-crate overrides. No claim of an entirely unsafe-free binary is warranted.

This was not a complete public-API leak analysis or whole-codebase reachability proof. No dependency or abstraction is recommended for deletion merely because it has one implementation.

## 3. Findings, ranked by leverage

### [DOC-01] Replace stale current-baseline assertions with dated evidence links

- **Evidence:** `README.md:12-15,56-57,156-159` says Clippy/checkpoint gates fail and calls this the latest recorded run. `docs/milestones/M83-baseline-integration.md:34-46` records a later isolated baseline and installation. The actual saved `audit/evidence/gate-integration-candidate-final.log:1114` contains `GATE PASS`; programmatically summing its test summaries gives **740 passed**. Saved locked real/offline logs also contain completed-check records. HEAD is `80b32eb`, after the documented `b57c8bf` integration.
- **Impact:** A new contributor is told a repaired historical baseline is currently red, while a maintainer could make the opposite error and apply isolated-candidate success to unrelated dirty changes.
- **Effort:** S (hours, including documentation/link review).
- **Risk:** LOW if claims are dated and scope-qualified; HIGH if rewritten as release certification.
- **Confidence:** HIGH that documents conflict; current dirty-tree gate outcome is **unverified by this reviewer**.
- **Fix sketch:** Replace duplicated pass/fail totals in README with a short safety/port-status statement and a pointer to the latest dated verification section. Preserve both historical failed output and later candidate evidence. Do not promote parent execution that has not finished.

### [DOC-02] Reconcile pre-fix F02/F10 descriptions with current source

- **Evidence:** `docs/design/epsilon-architecture.md:69-84` still says project rules are replaced and observations are process-wide knowledge. Current `crates/epsilon-cli/src/main.rs:1585-1589` retains project rules; `epsilon-safety/src/engine.rs:137-147` denies all native bash/shell calls in plan mode; `epsilon-core/src/exec.rs:337-391` authorizes both original capability and derived path subject. `epsilon-tools/src/read_log.rs:92-115,167-186` scopes observations and invalidates siblings. The latter ownership and plan-denial mechanisms were also confirmed in HEAD, not only in dirty files.
- **Impact:** Contributors may reimplement an already repaired ownership seam or troubleshoot a obsolete shell-prefix policy. README's `--plan` row (`README.md:81`) and CLI help (`main.rs:84,150-151`) give a different account from `docs/user/permissions.md:21-32`.
- **Effort:** S.
- **Risk:** LOW for narrow corrections; do not generalize native-tool denials into a side-effect classification guarantee for arbitrary MCP/extensions.
- **Confidence:** HIGH, source-verified; no tests rerun.
- **Fix sketch:** Make the user permissions guide the current behavior owner; update README/help/architecture to point to it or summarize the same limited contract. Mark F02/F10 old architecture descriptions superseded, retaining residual path-alias and extension-side-effect limits.

### [DOC-03] Make clean-checkout documentation closure explicit

- **Evidence:** The executed link scan below found eight tracked-source links to local-only destinations, including `docs/README.md:59` → untracked M84 and `docs/milestones/M7-compaction-and-prompts.md:20` → untracked M64. Forty-eight milestone documents are untracked at the scan snapshot.
- **Impact:** Local navigation success does not prove readers of a fresh checkout can follow current working-tree documentation. Blind staging would also mix unreviewed historical/source work into a docs fix.
- **Effort:** S for eight explicit links; M if reviewing the full historical untracked corpus for publication.
- **Risk:** MED because provenance and concurrent owner edits must remain intact.
- **Confidence:** HIGH at scan time.
- **Fix sketch:** For each local-only target, either explicitly select/review it for inclusion with its referring change, or label the reference as local pending evidence without presenting it as clone-available documentation. Preserve history; never manufacture dozens of milestone commits.

### [ARCH-01] Resolve contracts policy drift deliberately, not during layout cleanup

- **Evidence:** `AGENTS.md:75-79` prohibits new behavior in contracts and calls for reviewed extraction or policy amendment. `crates/epsilon-contracts/src/stream.rs:75-164` performs stateful assembly, including provider collision handling; `crates/epsilon-core/src/lib.rs:1698` uses it.
- **Impact:** Contributors cannot use “contracts is types-only” as a reliable placement rule; provider protocol exceptions can accumulate in the most widely depended-on crate.
- **Effort:** M, including consumer characterization and feature-matrix tests.
- **Risk:** MED/HIGH: moving or changing folding affects partial messages, tool calls and cancellation/replay behavior.
- **Confidence:** HIGH for policy drift, not a claim that current folding should be deleted.
- **Fix sketch:** After pi parity, decide via a narrow ADR whether vocabulary-level deterministic helpers are allowed and which provider-specific normalization belongs in llm. Preserve behavior first; do not create a new crate solely to host one helper.

### [DX-01] Document or centralize the two local dev-dependency pins

- **Evidence:** `AGENTS.md:85-88` and `Cargo.toml:25-27` require workspace-declared direct exact pins; `crates/epsilon-server/Cargo.toml:23-24` directly pins http-body-util and tower outside that table.
- **Impact:** A dependency review looking only at the declared central source misses these two direct test dependencies.
- **Effort:** S.
- **Risk:** LOW if versions/features remain identical and lockfile behavior is checked in a later authorized implementation slice.
- **Confidence:** HIGH.
- **Fix sketch:** Either explicitly permit crate-local exact test pins or move the declarations centrally without a version bump. No dependency removal is justified here.

### Lower-priority ownership contradictions, not new runtime findings

- `docs/README.md:73-77` says milestones are written once and current user facts have one owner; `AGENTS.md:166-169` asks logs/totals to move with ongoing work. Clarify that amendments append dated corrections, while live status lives in user guides/audit. Do not rewrite past recorded results into current success.
- `audit/README.md:64-66` and `audit/04-roadmap.md:10,54` retain Phase 1/2 approval-stop language, while later M83 records explicit implementation authorization. These are valid historical instructions only when visibly scoped to their date. The current direct task governs this audit. Add a future supersession note rather than silently editing the old authorization history.
- `AGENTS.md:178` says everything readers need is under docs, but its own table and `docs/README.md:17,26` legitimately keep audit and active agent memory outside docs. Prefer accurate audience wording over moving those trees for literal compliance.
- `README.md:171` calls reference snapshots “bundled,” whereas `docs/README.md:37` and `AGENTS.md:198` say they are not committed. Use “locally inspected snapshots”; a clone does not bundle them.

## 4. Strongest useful mechanism from each reference

These are targeted mechanism assessments, not rankings of whole products. All reference paths below are relative to the named local snapshot in `references/`; no upstream HEAD, live compatibility or reference test execution is asserted. **Exactly three candidates were inspected and compared. New adoption is deferred until pi parity.**

### DSH v0.1.1-rc.2 — byte-budgeted incremental retention with honest omission

**Reference evidence:** `packages/util/output-retention/src/index.ts:247-335` implements `TextRetainer`: prefix capacity, rolling suffix retention, per-push truncation decisions and total-byte accounting. `:350-385` handles contiguous no-omission decoding differently from true cuts and includes UTF-8 boundary trimming in omitted-byte totals. Its caller `packages/fs/tool-fs-search/src/search-core.ts:317-340` uses the retainer for byte-bounded line previews and bounded retained match items. `packages/util/output-retention/tests/output-retention.spec.ts:148-153,204-257` contains relevant UTF-8 split/cut assertions; inspected, not executed.

**Epsilon comparison:** `crates/epsilon-safety/src/sandbox.rs:259-279` drains every chunk into an ever-growing Vec while forwarding updates. `epsilon-tools/src/shell.rs:380-408` awaits those buffers, then `:410-413` combines and truncates them. The confined path likewise truncates after obtaining full outcome strings (`:301-305`). Display truncation therefore does not cap live retained bytes.

**Judgment:** This is DSH's highest-leverage narrowly transplantable mechanism for the inspected Epsilon seam: bound retention during ingestion and expose omitted-byte/spill disposition explicitly. Keep it inside the existing process/output ownership, not a port of DSH's package/service framework. Do not assume JS `subarray` implies allocation isolation: it can retain backing memory, and a Rust implementation should own bounded retained buffers. A retention helper alone also does not bound disk spills or own descendant/process deadlines.

**Post-parity acceptance:** bounded retained memory under ordinary finite chunk streams, multibyte boundaries, honest exact/unknown omission, and explicit spill failure/size policy; integrate once across native shell backends. No runtime benchmark was performed here.

### Hermes Agent 2026.8.19 — observer-scoped knowledge with shared mutation exclusion

**Reference evidence:** `tools/file_state.py:62-90` separates per-agent reads, global last writer and per-path locks. `:93-140` records observer knowledge and successful writer state with map caps; `:142-215` checks sibling writes, external changes and partial views. This is integrated, not just a utility: `tools/file_tools.py:1967-1969` records reads; `:2279-2304` locks/checks before writing and notes successful writes. The module returns diagnostics and callers determine enforcement; its timestamp/path design is not an OS containment guarantee.

**Epsilon comparison:** Current `crates/epsilon-tools/src/read_log.rs:92-115` uses opaque weak-owner scopes; `:167-186` marks sibling observations invalid even for equal-size/coarse-clock writes, refreshing only the writer. `epsilon-core/src/lib.rs:401,535-539` creates and resets observer lifetime, and `files.rs:302-314,477-501` calls the guard inside mutation paths. This important seam is **already integrated**, contradicting the older architecture text. Epsilon uses lifetime identity instead of copying Hermes's string task identity or wall-clock last-writer model.

**Judgment:** Retain and characterize the existing separation; do not port a second registry. Hermes is useful as a test/rationale reference for observer isolation, partial-read semantics and bounded active-state retention. Epsilon's per-active-owner path maps currently lack the explicit per-agent cap visible in Hermes (`read_log.rs:99,152`; reference `file_state.py:50-56,112,140`); capacity policy is a later design option, not an immediate bug claim. Eviction semantics must fail safely, not donate knowledge.

**Post-parity acceptance:** independent observers never inherit one another's reads, restore/reopen resets knowledge, successful own writes refresh only the writer, sibling writes invalidate, and retired owners are reclaimed. Resolved-object identity/TOCTOU hardening remains separate; neither reference proves race-resistant filesystem access.

### opencode 1.18.21 — edit-associated diagnostic freshness

**Reference evidence:** `packages/opencode/src/tool/edit.ts:196-211` touches the changed document and attaches diagnostic results to the tool outcome. `src/lsp/lsp.ts:351-356` carries the document version into waiting. `src/lsp/client.ts:577-582` increments versions; `:464-495` waits for matching path/server, rejects a reported wrong version, uses arrival time for unversioned pushes, debounces and cleans listeners/timers. This is a bounded freshness heuristic, not proof every language server obeys versions.

**Epsilon comparison:** `crates/epsilon-tools/src/files.rs:23-35,511,645` already appends optional diagnostics through `DiagnosticsProvider`; no new LSP feature scaffold is needed. `epsilon-lsp/src/client.rs:249-256` sends 1 for open and fixed 2 for every change; `:268-283` accepts a matching URI without checking a published version. Its freshness ownership therefore falls short of the inspected opencode client even though diagnostic output exists.

**Judgment:** Borrow version/freshness accounting after pi parity, keeping contracts vocabulary and the existing LSP adapter. Do not import opencode's full Effect/service/runtime or formatting/snapshot architecture to improve this seam. Diagnostic absence must not mean an edit was validated; a failed optional diagnostic must not erase a successful mutation outcome.

**Post-parity acceptance:** monotonic per-document versions, late earlier-generation diagnostics rejected or marked stale, explicit handling of servers that omit versions, a total deadline including writes, and cancellation-safe framing/client retirement. Existing F14 remains a correctness lead; no language server was launched by this review.

## 5. Executed local documentation-link check

### Scope and method

Scanned Markdown/MDX files discovered from tracked plus nonignored untracked paths, excluding any path component in `.git`, `.epsilon`, `.tmp-realprobe`, `target`, `references`, `.agents`, `node_modules`, `.venv`, `__pycache__`; also excluded `.env*`, `credentials`, `secrets`, `auth.json`. Thus both root `references/` and `audit/references/` were excluded. The checker did not read local runtime/credential files. It read root/docs/audit Markdown, active memory Markdown and the product agent-contract Markdown.

For each scanned document, fenced examples were masked without changing line offsets. Recognized explicit inline Markdown links/images, reference definitions and quoted HTML `src`/`href`; ignored schemes, network URLs and same-document fragments; URL-decoded paths, stripped query/fragment via URL parsing, and resolved relative paths against the referring file. A directory counted as tracked if it contained tracked descendants. Destination contents were not read.

Limitations: this is a filesystem-destination checker, not a full CommonMark renderer or anchor validator. Bare code spans/prose paths, heading fragments, shortcut reference-label resolution, unusual nested parentheses, remote URLs and reference trees were not certified. No “all documentation references are valid” claim follows from these results. A root-directory target convention was not needed by the links observed.

### Results before authoring this report

- **156** eligible Markdown/MDX source files scanned.
- **202** explicit local link occurrences examined.
- **194** link occurrences resolve to tracked files/directories.
- **8** link occurrences resolve only to local untracked files.
- **0** missing destinations expected to be tracked.
- **0** missing destinations absent from the tracked index (never-tracked/untracked candidates).
- **0** in-scope tracked files missing on disk.

All eight local-only link occurrences originate in tracked documents. They are **not filesystem-broken in this checkout**; they are **working-tree documentation closure gaps** if the referring edits are committed without their destinations. They do not prove that all eight links already appear in committed HEAD.

| Tracked source at scan time | Destination present locally but untracked |
|---|---|
| `agents/memory/README.md:40` | `agents/memory/f09-path-policy.md` |
| `agents/memory/README.md:41` | `agents/memory/port-revalidation.md` |
| `audit/02-defect-ledger.md:137` | `audit/evidence/f09-path-policy-map.md` |
| `audit/05-verification-log.md:422` | `audit/evidence/gate-f09-find.log` |
| `audit/05-verification-log.md:423` | `audit/evidence/gate-f09-find-repeat.log` |
| `audit/05-verification-log.md:423` | `audit/evidence/f09-find-review.json` |
| `docs/README.md:59` | `docs/milestones/M84-find-directory-resource.md` |
| `docs/milestones/M7-compaction-and-prompts.md:20` | `docs/milestones/M64-measured-pressure.md` |

`agents/memory/README.md:41` appeared during concurrent work after the initial read. This illustrates why these counts are a snapshot rather than an atomic repository export. This new report was intentionally not added to another index under the report-only instruction.

### Revalidation recipe

A later reviewer can reproduce the checker without builds or dependencies using Python stdlib:

1. Read NUL-delimited `git ls-files -z` and `git ls-files --others --exclude-standard -z` into sets; do not walk runtime trees.
2. Apply the exclusions above before opening any `.md`/`.mdx` source.
3. Mask fenced blocks with equal-length whitespace while preserving newlines. Extract explicit Markdown destinations, reference definitions, and quoted HTML image/link attributes.
4. Parse with `urllib.parse.urlsplit`, skip remote/empty paths, URL-decode and normalize against source parent with `posixpath`.
5. Classify `Path.exists()` independently from tracked membership; for directories, test tracked descendant prefix membership. Record source path/line and its tracked/untracked status too.
6. Aggregate counts in code, output all missing targets and all tracked-source→local-only edges. Never silently equate “exists” with “ships in a clone.” Exclude this evidence report from comparison with the original 156-source snapshot, or explicitly disclose its addition.

## 6. Restrained Rust/OSS layout proposal

**No moves were performed.** The current top-level shape is already conventional:

```text
README.md                 short product entry; limitations + links, no moving gate totals
LICENSE                   project license; retain attribution/provenance separately
CHANGELOG.md              user-visible release/change record
AGENTS.md                 contributor/agent rules
Cargo.toml, Cargo.lock    executable workspace, pinned dependency ownership
crates/<crate>/src/       implementation; stable public crate identities
crates/<crate>/tests/     integration/fixture ownership; unit tests stay beside modules
scripts/                  only maintained developer automation
assets/                   project presentation assets
.github/                  CI/repository metadata
docs/README.md            audience-oriented map
docs/user/                single owner of current product behavior
docs/design/              architecture and ADR intent, implementation status explicit
docs/history.md           chronological provenance
docs/milestones/          immutable dated narrative plus explicit factual amendments
docs/plans/, research/, qa/, handoff/  visibly dated/historical where appropriate
audit/                    dated findings/roadmap; evidence retains original provenance
agents/memory/            bounded active-work recovery, not a second product manual
.agents/skills/           local contributor procedures, not runtime code
```

Do not create empty `examples/`, `benches/`, `tests/` at root, `internal/`, `packages/`, or `xtask/` merely to look conventional. A future human-facing `CONTRIBUTING.md` could be a short navigation page if onboarding demonstrates a need; it should not duplicate AGENTS gate commands. Do not move private `.epsilon/`, ignored probes, targets or reference snapshots into tracked documentation.

### Preservation and source-move discipline for a later authorized pass

1. Correct reader-facing status and ownership in place first. Keep existing history/milestone numbering and source-origin comments.
2. Before any source extraction, preserve the exact dirty/untracked boundary and establish behavior characterization. Group by responsibility (TUI session commands/editor/rendering, CLI argument/mode composition, core lifecycle), not a target line count. Do not infer dead code from size or single implementations.
3. For real moves, record `old path → new path`, originating commit/snapshot and reason; update local links and citations while retaining the historical citation's date. Use explicit path staging and review only owned changes. Do not invent past milestone commits to make history look tidy.
4. Keep a source/license provenance ledger for any future port: upstream project, inspected version/commit, original file/symbol, copied versus independently implemented behavior, local destination, applicable notices/license obligations and verification evidence. The current project MIT declaration and README acknowledgments do not by themselves certify third-party port provenance.
5. Separate the post-pi ADR decision on contracts behavior from documentation cleanup. Retain existing libraries and adapters until a measured ownership problem and consumer tests justify a change.

### Over-engineering disposition

`shrink:` Remove duplicate **current** pass/fail/permission assertions from README/architecture in favor of maintained owner links; retain dated historical evidence. [`README.md:12-15,81,156-159`; `docs/design/epsilon-architecture.md:69-84`]

No source/dependency deletion is established by this review. **Net certified code cuts: 0 lines, 0 dependencies.** A speculative negative line count would be misleading; the useful reduction is contradictory status ownership, not amputating working abstractions.

## 7. Verification limits and handoff

**Executed here:** read-only Git inventory/HEAD checks, manifest parsing, local link scan/classification, targeted source inspection, and programmatic aggregation of saved gate summaries. Twelve manifests, three reference mechanisms and all 202 recognized local link occurrences in the stated snapshot were accounted for.

**Recorded only:** the saved isolated-candidate gate pass and 740-test aggregate, install/cleanup narrative in M83, and prior audit test findings. They do not certify this concurrently changing working tree. CI source defines Windows full gates, Linux confinement/full tests and an MSRV lane (`.github/workflows/ci.yml:21-92`), but no CI run or MSRV/platform result was checked here.

**Not done:** pi parity assessment, source/API exhaustive audit, live-provider/LSP/reference execution, builds/tests/installs, dependency advisory scan, fresh clone, anchor/remote-link validation, secret inspection, license certification, restructuring or commits. Parent build/cache state was left alone.

**Issues encountered:** one guessed shell module name was absent and was corrected to `shell.rs`; a search-pattern parser rejected one regex, so subsequent targeted reads used Python string matching. Neither blocked the audit. Concurrent document edits limit atomicity; re-run closure checks against the parent's eventual selected index before committing. New reference adoption remains deferred until pi parity.
