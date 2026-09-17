# HEAD dependency-closure review

Date: 2026-09-17. Scope: static, read-only source/history audit; this report is the only repository write. No source edits, staging, commits, builds, installs, tests, network-provider calls, or exploit reproductions were performed.

## Verdict

**HEAD does not have source dependency closure. Do not describe it as a verified clean-checkout build.** Seven unconditional core module declarations have neither legal module-file location in HEAD. Other committed callers require untracked modules, missing module declarations, unpublished APIs in dirty tracked files, and missing direct dependency/feature wiring. Adding the seven files alone is not sufficient.

These are static source-resolution findings, not compiler diagnostics from a clean checkout. No clean-checkout build was attempted, and the inventory below is a bounded set of demonstrated blockers, not an exhaustive compiler-error list or a proven minimal patch.

## Snapshot and method

- HEAD: `350c9ecabf5bbe28fb12f0ce86a01f9ed696e8bd`, following F02 `a160b8d` and F10 `e778ab3`.
- `git ls-tree -r --name-only HEAD`: 268 tracked files. `git diff --cached --name-only`: empty; no staged changes.
- `git ls-files --modified`: 75 modified tracked paths. `git ls-files --others --exclude-standard`: 120 individual untracked files, 54 beneath `crates/`. Directory-collapsed status has 102 untracked entries, not 102 files. No HEAD-tracked file was absent from disk.
- Inspected committed blobs with `git show HEAD:<path>`, compared working copies and manifest diffs, checked candidate module paths against the HEAD tree, and parsed existing Cargo.lock files as TOML without invoking Cargo.
- Unless prefixed **WT**, line references below refer to **HEAD**, not the possibly different working copy. Absence claims were checked against the committed tree/blob, not inferred merely from dirty status.
- Read `AGENTS.md`, `.agents/skills/ponytail-review/SKILL.md`, `agents/memory/README.md`, `agents/memory/closeout.md`, and the roadmap. Ponytail's complexity-only pass is not this dependency-correctness review; no deletion recommendation or invented line-saving metric is appropriate.

## 1. Untracked modules required directly by committed source

| Committed reference | Missing HEAD path / working-tree evidence |
|---|---|
| `crates/epsilon-core/src/lib.rs:10` — `pub mod agents;` | `crates/epsilon-core/src/agents.rs` is untracked. |
| Same file `:11` — `mod cache_stats;` | `crates/epsilon-core/src/cache_stats.rs` is untracked. |
| Same file `:16` — `mod guards;` | `crates/epsilon-core/src/guards.rs` is untracked. |
| Same file `:17` — `pub mod paths;` | `crates/epsilon-core/src/paths.rs` is untracked. |
| Same file `:19` — `pub mod references;` | `crates/epsilon-core/src/references.rs` is untracked. |
| Same file `:22` — `mod untrusted;` | `crates/epsilon-core/src/untrusted.rs` is untracked; `crates/epsilon-core/src/exec.rs:745,752` also calls it. |
| Same file `:23` — `mod verify;` | `crates/epsilon-core/src/verify.rs` is untracked. |
| `crates/epsilon-tools/src/files.rs:14` imports `epsilon_contracts::diagnostics::DiagnosticsProvider` | `crates/epsilon-contracts/src/diagnostics.rs` is untracked; HEAD contracts `src/lib.rs:10–17` also lacks its declaration. WT declaration is `:12`. |
| `crates/epsilon-tools/src/files.rs:115` calls `crate::image::detect_supported_image_mime` | `crates/epsilon-tools/src/image.rs` is untracked; declaration exists only in WT tools `src/lib.rs:13`. |
| `crates/epsilon-tools/src/files.rs:292,467` calls `crate::mutation::with_file_mutation_queue` | `crates/epsilon-tools/src/mutation.rs` is untracked; declaration exists only in WT tools `src/lib.rs:14`. |
| `crates/epsilon-cli/src/main.rs:1908,1915` uses `epsilon_cli::doctor::{Inputs,run}` | `crates/epsilon-cli/src/doctor.rs` is untracked; HEAD CLI `src/lib.rs:3–4` declares only rpc_mode/runtime. WT `:3` adds doctor. |
| `crates/epsilon-cli/src/main.rs:709,799` uses `epsilon_llm::oauth_login` under `real` (`:707,798`) | `crates/epsilon-llm/src/oauth_login.rs` is untracked and HEAD LLM `src/lib.rs:11–22` does not declare it; WT `:20` does. |

For all seven core declarations, both `<module>.rs` and `<module>/mod.rs` are absent from HEAD. This alone establishes broken source closure on the ordinary core path, independent of feature-specific findings.

## 2. Files already tracked are not enough: declarations and APIs remain dirty

| Committed caller | Missing committed companion; existing WT implementation |
|---|---|
| `crates/epsilon-tools/src/files.rs:189,302,314,477,501,635` uses `crate::read_log`; `:305,482` uses `crate::checkpoint` | Both files are tracked in HEAD, but HEAD tools `src/lib.rs:9–14` declares neither module. WT `:9,15` wires them. Do not misreport these two files as untracked. |
| `crates/epsilon-tools/src/files.rs:525` matches `EditOutcome::Suspicious` | HEAD `crates/epsilon-tools/src/edit_match.rs` lacks that variant; dirty WT defines it at `:79`. |
| `crates/epsilon-core/src/lib.rs:370,1367,1374,1434` uses `UsageAnchor`, `anchored_tokens`, `legal_cut_point` | HEAD `crates/epsilon-core/src/compact.rs` lacks these items; WT defines them at `:30,46,73`. Existing MIN/MAX_TAIL_TOKENS are **not** missing (`HEAD compact.rs:46–47`). |
| `crates/epsilon-core/src/lib.rs:931,1209,1250` emits `AgentEvent::Notice` | HEAD `crates/epsilon-contracts/src/agent_event.rs:105` enum lacks Notice; WT adds it at `:109`. Downstream exhaustive event matches also need reviewed companion changes. |
| `crates/epsilon-core/src/lib.rs:2126–2127` uses `catalog::resolve_model` and `ModelMatchKind::Partial` | HEAD LLM catalog lacks those declarations; WT `crates/epsilon-llm/src/catalog.rs:160,197` defines them. `credentials_ready` already exists in HEAD and is not a missing symbol. |
| `crates/epsilon-cli/src/main.rs:1099,1557,1570,1620–1621` uses runtime trust/resource APIs | HEAD `crates/epsilon-cli/src/runtime.rs` lacks `ProjectTrustStore`, `skill_roots_trusted`, `extension_roots_trusted`, `agent_user_roots`, and `agent_project_roots`; WT declarations are `:261,575,600,621,630`. |
| `crates/epsilon-cli/src/main.rs:587,590` matches `StoredCredential::Api` / `Oauth`; `:921` calls `save_credential_full` | HEAD `crates/epsilon-llm/src/auth.rs:24` defines StoredCredential as a struct, not this enum, and lacks the full-save API. WT enum starts at `:49`, full-save function at `:778`. This is not solely a feature-gated OAuth module omission. |
| `crates/epsilon-cli/src/main.rs:1606` calls `SessionStore::open_in` | HEAD `crates/epsilon-session/src/store.rs` lacks it; WT defines it at `:213`. Review session header/lease/discovery companion changes together rather than importing this method in isolation. |
| `crates/epsilon-cli/src/main.rs:140,375,1547` uses `Lsp`, `Confinement::ReadOnly`, `builtin_registry_with` | HEAD tools `src/lib.rs:26–37,57–80` exposes the earlier confinement/registry API. WT adds the required items at `:75,46,150` respectively. |

## 3. Manifest, feature, and transitive companion closure

### Direct committed-code failures

- **Undeclared external crate:** `crates/epsilon-tools/src/files.rs:132–133` calls `base64::Engine` / `base64::engine`. HEAD tools `Cargo.toml:9–20` does not declare base64, nor does HEAD workspace dependency table (`Cargo.toml:26–53`). WT adds the exact pin at root `Cargo.toml:63` and the direct dependency at tools `Cargo.toml:21`. A transitive lockfile package is not a Rust direct dependency declaration.
- **Real feature not forwarded:** CLI `src/main.rs:28–29` refers to `epsilon_core::REAL_PATH_ENABLED` under CLI `real`. Core `src/lib.rs:43–44` exposes that marker only under core `real`. HEAD CLI `Cargo.toml:16` forwards only `epsilon-llm/real`; core `Cargo.toml:9–10` has its own feature. WT CLI `Cargo.toml:30` forwards llm/core/tui. HEAD also has no default-real declaration; WT `:29` enables it. Do not confuse default behavior with the explicit real-path compile dependency.

### Required follow-through if integrating existing owner companion files

- **LSP:** WT tools `src/lib.rs:91–108` connects diagnostics to `epsilon_lsp::ServiceConfig` / `DiagnosticsService` and `epsilon_safety::scrub_secret_env`; WT doctor `src/doctor.rs:277` also calls LSP. The entire `crates/epsilon-lsp/` crate is untracked. Its manifest (`WT :10–19`) depends on contracts/serde/serde_json/async-trait/tokio/tempfile. Root membership (`WT Cargo.toml:12`) and CLI/tools path dependencies (`WT Cargo.toml:37` / `:13` in their respective crates) are dirty. HEAD does not directly name epsilon-lsp in these manifests: this is the transitive completion of the committed Lsp/doctor interfaces, not a missing HEAD workspace member declaration.
- **OAuth/provider integration:** WT LLM `src/lib.rs:13,19–20,23–26` introduces untracked config_value/oauth/oauth_login/sanitize modules. WT oauth_login `:255–271,285` requires base64/sha2/getrandom. The LLM manifest additions and root getrandom pin remain dirty. Provider registration has additional untracked bedrock/github_copilot/mistral/openai_responses/vertex files. Review registrations, credential shape, provider callers, and tests as a dependency-connected slice; copying only lib.rs creates further missing modules.
- **Tools:** importing WT tools lib.rs also requires untracked find/task/webfetch files and embedded descriptions: `WT src/find.rs:31`, `WT src/task.rs:71`, `WT src/webfetch.rs:16` include `descriptions/find.txt`, `task.txt`, `webfetch.txt` respectively. Reqwest and base64 additions are in the dirty tools manifest.
- **TUI/session/safety:** proposed WT integration has untracked TUI keybindings/latex/trust_store, session discover, and safety env/escalation companions. TUI manifest adds epsilon-session and unicode-segmentation. These are candidate follow-through dependencies of the broader WT slices, not seven extra unconditional missing HEAD core declarations.
- **Lockfile must accompany reviewed manifests:** HEAD `Cargo.lock:556,620,695,712` records CLI/LLM/tools/TUI without the added edges; it has no epsilon-lsp package. WT package records at `:627,692,710,782,802` include those edges. This audit parsed the records; it did not resolve or regenerate a lockfile. Root reqwest TLS-feature changes also remain dirty and require normal review, not an assumption of runtime verification here.

## 4. Historical attribution: not all breakage originated in F02/F10

- Comparing `cda276e` (F02 parent) with `a160b8d` shows the seven core declarations entered with F02's whole-file core update. F02's source delta is much broader than the ownership fix: commit statistics show core lib.rs changed by approximately 1,700 lines.
- Comparing `a160b8d` with `e778ab3` shows the Lsp and doctor calls entered CLI main.rs with F10's whole-file update. The core exec and CLI main changes are broader than the authorization fix alone.
- **The files.rs diagnostics/image/mutation references already exist at `cda276e:crates/epsilon-tools/src/files.rs:14,115,292,467`.** Therefore this report does not attribute every closure defect to F02, nor claim the F02 parent was clean/buildable.
- `350c9ec` adds only AGENTS/memory documentation. Rewriting F02/F10 would neither be authorized nor necessarily repair older incomplete integration.

## 5. What existing execution evidence actually verifies

- Read and mechanically aggregated `audit/evidence/gate-f10-final2.log`: 55 test summaries total **794 passed, 0 failed, 1 ignored**. Gate exit records are at `:3` (fmt), `:254` (clippy), `:1591` (test), `:1618` (demo), `:1648` (real-check); `:1650` says GATE PASS.
- The log in HEAD equals its working copy line-for-line. Committing a successful log does not commit the source tree that produced it.
- `agents/memory/closeout.md:5–10` explicitly identifies that gate run as a **dirty working-tree** run and warns about untracked companions. The static comparison confirms why the distinction matters.
- Existing untracked install log `audit/evidence/f10-install.log:270–272` records release completion and executable replacement. Closeout `:15–18` records installation/demo exit success and points to the installed-demo log. These are prior working-tree/installed-binary results, not execution performed by this audit and not evidence of HEAD reproducibility.
- No build/test failure output was produced here; do not convert predicted source-resolution failures into invented compiler messages. No compiler validation, complete conditional-module resolver, or full transitive API review was performed.

## 6. Minimal forward-only integration plan (proposal; approval required)

1. **Freeze scope with the owner.** Preserve HEAD and all 75 dirty tracked / 120 untracked individual files through the owner's chosen backup process. Record explicit candidate paths and hashes before later integration. Do not stash/drop, reset/clean, mass-stage, amend, rebase, or reconstruct M33–M79 history. Roadmap `audit/04-roadmap.md:16` and `AGENTS.md:118–123` require preservation and explicit-path discipline.
2. **Review the smallest dependency-connected completion of already committed behavior.** Start with the concrete roots in sections 1–3, then expand only when a selected hunk needs another module/API/asset/manifest entry. Include declarations as well as files: tracked read_log/checkpoint, untracked diagnostics/image/mutation/core modules/doctor/OAuth, compact/registry/config/prompt and contracts companions, runtime/session/auth/catalog APIs, and the feature/direct-dependency fixes. Do not stage entire lib.rs/runtime.rs files merely because one required hunk is there.
3. **Use review batches, not fake historical milestones.** Review contracts/session/core support; tools/diagnostics/LSP support; LLM/credential/provider support; then CLI/TUI composition and exhaustive event consumers. These batches may cross-depend; do not publish knowingly non-closed intermediate commits just to keep a small file count. Prefer one honestly named forward baseline-integration commit for the approved connected component, or multiple commits only where each candidate is demonstrably closed. Keep F02 ownership and F10 additive plan-mode enforcement and their regression tests unchanged unless separately reviewed.
4. **Reconcile code, tests, dependencies and docs atomically.** For each accepted slice include its explicit regression/fixture paths and embedded description files, relevant ADR/milestone provenance, exact dependency pins, feature forwarding, and the matching Cargo.lock delta. Preserve unselected owner work in place. Root/tools/CLI LSP wiring requires the reviewed crate and contracts together; do not import every untracked milestone, reference archive, local skill, or runtime session merely to make status clean. Add this report to the docs/memory indexes in a separately approved documentation edit; this audit intentionally did not edit those owner files.
5. **Only then validate an isolated candidate tree containing exactly the approved tracked snapshot.** A later authorized integration task should materialize a separate checkout/export, verify no source leaks in from this dirty working tree, run locked dependency checks and the repository gate battery, including explicit real and offline feature paths, and record the candidate tree/commit hash plus actual outcomes. This is future work, not a claim of execution. Keep offline faux-provider tests; no paid calls or exploit reproductions are needed. Follow installation/cleanup rules only when that future task reaches verified completion.
6. **Publish forward only after candidate verification.** Explicitly review the staged diff against the agreed allowlist; obtain required owner approval; commit the verified snapshot without unrelated work. Retain the clean-checkout caveat until that exact committed tree is exercised successfully. Report any remaining dirty work rather than treating cleanliness as the acceptance criterion.

The minimum safe unit is dependency closure, not the seven obvious files and not the entire working tree. This audit provides concrete roots and boundaries for that review; it does not authorize integration or promise that this bounded list is a sufficient patch.

## Audit limitations / tooling notes

Two initial read-only collection attempts failed (native Python could not read an MSYS `/tmp` path; `git ls-files --name-only` is invalid). They were replaced with valid Git queries and native Windows paths. An early broad disk inventory included ignored references/runtime files and mislabeled non-HEAD files; it is **not used** for tracked/untracked counts or findings. Final counts come from Git's cached/modified/others inventories excluding ignored files. No ignored references or user runtime data are proposed for integration.
