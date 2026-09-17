# Pi surface fidelity and inherited TUI readiness — independent revalidation

**Baseline:** source-only review of the working tree over `80b32eb`, 2026-09-17. Reference: bundled `references/pi-0.84.2/`. Findings describe the inspected working tree, not just committed HEAD. No builds, installs, provider calls, credential access, exploit reproduction, staging or commits performed. Only this report is written.

**Read confirmed:** `AGENTS.md`; `agents/memory/README.md`, `next-hardening.md`, `f09-path-policy.md`; audit-playbook sections **1, 4, 8 and Finding format**. The existing audit/roadmap was read as historical context, not accepted as evidence. Actual implementations and dispatch paths on both sides were inspected. Review skill loaded; its mutation/build/stash workflow does not apply to this explicitly read-only delegation.

**Boundary:** TUI/input, tools, resource loading, extension surface and provider availability. Core-loop semantics, session projection/tree correctness and RPC internals belong to the other reviewer. No claim of exhaustive provider protocol or visual-terminal validation.

For citations below, **`pi/` expands exactly to `references/pi-0.84.2/`**; all other paths are repository-relative. Confidence is source confidence, never an executed conformance result.

## Findings (8)

### [SURFACE-01] Preserve attachment identity through file completion

- **Classification:** inherited TUI integration defect; missing pi completion behavior.
- **Evidence:** `pi/packages/tui/src/autocomplete.ts:107–120,407–423` builds completion values retaining `@`, quotes paths containing spaces and distinguishes directory continuation. `crates/epsilon-tui/src/app.rs:1029–1047` instead supplies a bare path to `at_insert`, replacing the entire `@fragment`. The submission path calls the reference-aware message constructor at `app.rs:893–897`, while `crates/epsilon-core/src/references.rs:113–115` expands reference tokens, not arbitrary bare paths. `crates/epsilon-tui/src/app.rs:6314–6366` tests the bare-path insertion/closed popup, not the submitted attachment.
- **Impact:** typing a complete `@file` and selecting that same file from the popup produce different messages: completion removes the attachment trigger, so the model receives a path rather than the attached content. Spaces/quoted paths and directory continuation are also not pi-equivalent. This is particularly misleading because the new popup and existing attachment constructor each appear functional in isolation.
- **Effort:** M, including end-to-end input fixtures.
- **Risk:** MED — editor tokenization and attachment parsing must agree without broadening permitted paths.
- **Confidence:** HIGH.
- **Fix sketch:** use one completion/token representation that retains attachment identity and supported quoting; test type → select → submit against manual reference submission. Include spaced/non-ASCII paths, directory continuation and cancellation. The local scan also spends its 200-file budget before filtering (`app.rs:963–1025`); explicitly characterize larger repositories before accepting this popup as complete.

### [SURFACE-02] Complete resource commands instead of substituting explanatory notices

- **Classification:** missing port scope plus an inherited misleading `/settings` instruction.
- **Evidence:** `pi/packages/coding-agent/src/modes/interactive/interactive-mode.ts:5727–5735,5780–5795` reloads the session/resources, keybindings, registered themes, runtime settings and autocomplete, with busy guards. `crates/epsilon-tui/src/app.rs:1805–1846` reloads skills, agents and keybindings only; nevertheless `app.rs:2185–2186` tells users to edit model/favorites TOML and `/reload` to apply without restarting. `pi/packages/coding-agent/src/core/prompt-templates.ts:57–102,194–203` implements named prompt resources and positional/default/sliced argument substitution. Epsilon's fixed command dispatcher (`crates/epsilon-tui/src/picker.rs:536–615`, `app.rs:1479–1485`) has no template resolution; targeted `PromptTemplate`/`substituteArgs` searches across `crates/` returned no matches. `pi/packages/coding-agent/src/core/package-manager.ts:978–1029,1032–1049` installs/removes/updates scoped resources; Epsilon's executable modes (`crates/epsilon-cli/src/main.rs:178–193`) expose no corresponding package lifecycle.
- **Impact:** model/favorite edits are not applied by the advertised command; prompt-command and installed resource workflows cannot be carried across from pi. Settings and scoped-model command names are not equivalent to pi's actual configuration selectors. A directory appearing in trust detection does not establish that it loads.
- **Effort:** L — multiple bounded slices, not a notice-text-only fix.
- **Risk:** HIGH — reload crosses runtime settings, resource ownership, trust and extension lifecycle.
- **Confidence:** HIGH for the cited command gaps; MED for the broad package absence, based on dispatcher/source inventory rather than execution.
- **Fix sketch:** first correct the inaccurate instruction, then introduce a shared startup/reload resource snapshot with truthful per-resource diagnostics. Port local templates, config application and scoped selection before package install/update/remove. Keep package sources and trust scopes explicit. The current submit trust gate (`app.rs:1078–1137`) blocks ordinary commands while review is pending: this report does **not** claim that `/reload` alone bypasses that gate.

### [SURFACE-03] Port skill discovery and YAML semantics rather than claiming collision parity

- **Classification:** partial implementation; explicit “mirroring pi” claim contradicts the reference.
- **Evidence:** `pi/packages/coding-agent/src/core/skills.ts:160–170,191–220,255–270` supports a root `SKILL.md`, root Markdown files and recursive skill discovery. `crates/epsilon-core/src/skills.rs:63–81` scans immediate child directories only. Pi parses frontmatter and surfaces validation diagnostics (`pi/packages/coding-agent/src/core/skills.ts:283–316`); Epsilon's flat colon splitter and silent rejection (`crates/epsilon-core/src/skills.rs:36–56,88–102`) do not implement YAML quoting or multiline scalars. Pi retains the first same-name skill and reports winner/loser paths (`pi/packages/coding-agent/src/core/skills.ts:399–432`); Epsilon replaces the earlier result despite its parity comment (`crates/epsilon-core/src/skills.rs:65,104–108`).
- **Impact:** common quoted names or grouped skill collections disappear, descriptions are misread, and duplicate names select a different skill without a diagnostic. Shared skill collections cannot be treated as portable merely because their directories are detected.
- **Effort:** M–L, including migration/precedence fixtures.
- **Risk:** MED — correcting precedence changes which existing skill wins.
- **Confidence:** HIGH.
- **Fix sketch:** port discovery/frontmatter/collision behavior with a fixture corpus from implemented pi cases. If project-over-global precedence is intentionally retained, document it as an accepted divergence and expose the winning source; remove the exact-parity claim. Preserve Epsilon's trust gate rather than importing unrestricted discovery wholesale.

### [SURFACE-04] Restore usable clear/exit and keybinding override semantics

- **Classification:** partial keyboard port and inherited misleading effective-binding surface.
- **Evidence:** `pi/packages/coding-agent/src/modes/interactive/interactive-mode.ts:3797–3809` clears on first Ctrl+C, exits on a quick second press, and handles Ctrl+D exit only when the editor is empty. `crates/epsilon-tui/src/keybindings.rs:27–48` declares both `app.clear=ctrl+c` and `app.exit=ctrl+c/ctrl+q`; `crates/epsilon-tui/src/app.rs:3193–3195` exits immediately, and a targeted search found no `app.clear` dispatch. Pi respects an explicitly empty binding array and records conflicts (`pi/packages/tui/src/keybindings.ts:247–266`); Epsilon drops empty arrays and falls back to defaults (`crates/epsilon-tui/src/keybindings.rs:112–124,160–172`). Epsilon's effective map also prints user-only IDs (`:217–245`) even though many editor actions remain hard-coded (`app.rs:3747–3811`).
- **Impact:** a pi user's habitual clear gesture closes the application instead; declared clear remaps do nothing. Users cannot disable a binding with `[]`, and `/hotkeys` can list configurations that are not implemented actions. The inherited remapped-quit test (`app.rs:5595–5618`) does not cover clear behavior or empty-array disabling.
- **Effort:** M.
- **Risk:** MED — modifier handling and modal precedence must be characterized on supported terminals.
- **Confidence:** HIGH.
- **Fix sketch:** map supported action IDs through a single dispatcher, preserve explicit empty overrides and report unsupported IDs separately from effective bindings. Port pi clear/exit behavior, or obtain an explicit owner decision on the alternative; Ratatui itself does not require different keyboard semantics. Add app-level default/remapped/disabled/modal tests, not only manager lookup tests.

### [SURFACE-05] Complete paged reads beyond the initial byte prefix

- **Classification:** user-visible tool contract divergence; not a reopening of the fixed destructive-edit bug.
- **Evidence:** `pi/packages/coding-agent/src/core/tools/read.ts:275–295,302–317` selects from the requested line offset before limiting output and returns continuation offsets. `crates/epsilon-tools/src/util.rs:204–217` reads only the first 256 KiB; `crates/epsilon-tools/src/files.rs:144–157` then applies the requested line offset to that prefix. Epsilon tells the model to use grep/bash for the remainder (`files.rs:176–187`) rather than permitting read pagination. Its strict edit reader now explicitly refuses over-limit/invalid UTF-8 source (`util.rs:171–191`), so the old claim that edits overwrite a truncated prefix is not supported by current source.
- **Impact:** a large normal text file cannot be read to completion through repeated `read(offset=...)` calls, even when each requested page is small. Epsilon also conflates empty and out-of-range reads, whereas pi raises a distinct offset error. A larger output cap alone does not deliver the missing workflow.
- **Effort:** M.
- **Risk:** MED — observation completeness and mutation guards depend on read-window metadata.
- **Confidence:** HIGH.
- **Fix sketch:** implement bounded streaming line skipping/page collection, accurate continuation and out-of-range responses while keeping strict edit refusal until separately designed. Test pages starting beyond 256 KiB, long lines, multibyte boundaries and explicit small limits. Do not copy pi's full-file buffering merely to achieve page semantics.

### [SURFACE-06] Bridge extension commands and UI capabilities across process isolation

- **Classification:** missing SDK behavior, distinct from an accepted implementation-language/process divergence.
- **Evidence:** `pi/packages/coding-agent/src/core/extensions/loader.ts:264–306,398–410` implements tool, command, shortcut, renderer and provider registrations; registered commands reach autocomplete in `pi/packages/coding-agent/src/modes/interactive/interactive-mode.ts:707–713`. `crates/epsilon-ext/src/manifest.rs:18–37` declares process identity, tools and event subscriptions only. `crates/epsilon-ext/src/host.rs:164–186,244–263` invokes tools and accepts text/details results, ignoring other message kinds. `docs/design/epsilon-decisions.md:254–271` deliberately chooses isolated processes and declarative widgets instead of direct components; it does not make the absent declarative UI implementation exist.
- **Impact:** pi extension workflows that register a command, shortcut, picker/widget or provider cannot be expressed by the current host protocol. Tool/event fixture success cannot establish extension SDK parity, and raw text tool results cannot carry the complete typed output surface.
- **Effort:** L, staged protocol/API work.
- **Risk:** HIGH — lifecycle, capability checks, cancellation and frontend ownership cross the boundary.
- **Confidence:** HIGH for the inspected registration/host gap.
- **Fix sketch:** retain isolated processes; enumerate required public capabilities and port a versioned declarative protocol, beginning with commands, invocation context and typed results. Add host-to-TUI fixtures for registration, removal/reload and collision diagnostics, then selectors/widgets and provider registration. Direct TypeScript component ABI compatibility is not implied.

### [SURFACE-07] Add the actual Codex subscription provider, not only Codex-named models

- **Classification:** missing high-value provider workflow; not a catalog-count finding.
- **Evidence:** `pi/packages/ai/src/providers/openai-codex.ts:7–20` composes ChatGPT subscription OAuth, Codex models and the dedicated API. `pi/packages/ai/src/api/openai-codex-responses.ts:262–279,630–639` prepares account-aware headers and dedicated Codex HTTP/WebSocket URLs. `crates/epsilon-llm/src/providers/mod.rs:88–193` routes its finite wire set and custom providers to generic OpenAI-compatible completion transport, with no Codex subscription wire. `crates/epsilon-llm/src/oauth_login.rs:135–139,191–222` explicitly defers Codex while exposing the implemented login list.
- **Impact:** users with a ChatGPT subscription cannot reproduce pi's login → select Codex → run workflow. Generic Responses support or models containing “codex” in their names does not supply the subscription transport/auth behavior.
- **Effort:** L.
- **Risk:** HIGH — account/auth metadata, transport differences and service compatibility need coordinated validation.
- **Confidence:** HIGH for missing dedicated integration; live compatibility unverified.
- **Fix sketch:** port provider identity, model metadata, subscription login and dedicated transport together using local wire/auth fixtures. Keep provider availability truthful until that slice is wired through CLI/TUI. Inventory other pi adapters separately; this bounded review does not claim exhaustive provider absence or refreshed-token correctness.

### [SURFACE-08] Use typed image blocks and implement the screenshot-input workflow

- **Classification:** missing image-input scope plus inherited incorrect fallback heuristics.
- **Evidence:** `pi/packages/coding-agent/src/modes/interactive/interactive-mode.ts:2870–2888` reads an image from the clipboard before text fallback. Epsilon's clipboard helper and action (`crates/epsilon-tui/src/app.rs:2811–2814,3910–3926`) read text only. `crates/epsilon-tools/src/files.rs:124–138` produces a typed `ContentBlock::Image`, but the TUI filters all non-text blocks out before calling its image heuristic (`crates/epsilon-tui/src/state.rs:433–467`; restored rendering repeats this at `:300–339`). Pi renders explicitly typed image blocks (`pi/packages/coding-agent/src/modes/interactive/components/tool-execution.ts:341–363`). Epsilon instead classifies a long predominantly alphanumeric single text line as an image and replaces both collapsed and expanded content (`state.rs:54–86,459–467`); tests explicitly assert that replacement for a bash text result (`:1014–1033`).
- **Impact:** pasting a screenshot cannot attach it; real typed images do not receive the claimed fallback treatment; long ordinary tool text can be hidden as an image even when expanded. This is not equivalent to honestly deferring Kitty/iTerm rendering.
- **Effort:** M for correct typed fallbacks/text preservation; L for bounded clipboard/import/resize plus terminal graphics.
- **Risk:** MED — clipboard/platform capability failures and attachment byte limits need explicit handling.
- **Confidence:** HIGH.
- **Fix sketch:** recognize typed images, preserve text verbatim in expansion and remove content-guessing as a replacement for type handling. Add typed image-only and mixed text/image fixtures, plus long non-image text. Implement bounded screenshot ingestion and resize before graphics; expose terminal capability fallback explicitly.

## Accepted divergences versus unfinished scope

- **Keep Rust/Ratatui**, per `docs/design/epsilon-decisions.md:360–369`, rather than replacing the frontend with pi's component runtime. This permits different rendering implementation, not silently different clear/submit/completion behavior.
- **Keep isolated extension processes and no direct component ABI**, per ADR-009 `:254–271`. Commands, typed results and the promised declarative widgets remain missing work, not waived scope.
- **Keep declarative TOML/project trust**, per ADR-012 `:394–427`, rather than copying pi's configuration file format. Settings breadth, truthful reload, source precedence and resource diagnostics still require a port.
- **Keep bounded ingestion and strict edit refusal** (`crates/epsilon-tools/src/util.rs:171–191`) while implementing useful pagination. Do not restore destructive behavior or unbounded reads to imitate pi's mechanism.
- **Not accepted merely because a comment says “deferred”:** clipboard images, terminal graphics, themes, local templates, package management, extension UI and missing providers. Epsilon's fixed theme (`crates/epsilon-tui/src/theme.rs:1–8`) contrasts with pi's theme/resource settings (`pi/packages/coding-agent/src/modes/interactive/interactive-mode.ts:4437–4439,5787–5789`). These remain visible in the completion matrix unless the owner explicitly accepts narrower scope.

## Inherited-diff readiness (not blanket approval)

Read-only `git diff --numstat -- crates/epsilon-tui` returned:

| Tracked dirty path | Added / removed lines |
|---|---:|
| `Cargo.toml` | 4 / 0 |
| `src/app.rs` | 2735 / 165 |
| `src/editor_ops.rs` | 199 / 6 |
| `src/fuzzy.rs` | 133 / 6 |
| `src/lib.rs` | 3 / 0 |
| `src/markdown.rs` | 20 / 0 |
| `src/picker.rs` | 760 / 74 |
| `src/state.rs` | 355 / 5 |

`lib.rs:18–27` also introduces companion keybinding, LaTeX and trust-store modules. Therefore neither these numstats nor HEAD's earlier gate totals cover all inherited dependencies. No inherited file was edited.

**Useful implemented work to retain for selective integration:** grapheme-aware movement/deletion (`editor_ops.rs:16–72,143–157`) with cluster tests; fuzzy token filtering (`fuzzy.rs:156–191`) and picker fixtures; external-editor injected round-trip tests (`app.rs:5330–5390` vicinity); notices and partial-reply error rendering (`state.rs:404–420,550–558`). These are source-inspected positives, not proof of complete pi algorithms or executed native behavior.

**Hold as an undifferentiated merge:** findings 01, 02, 04 and 08 are inside the large inherited surface. Existing tests sometimes validate the local implementation rather than the user workflow (bare-path completion and text-as-image are concrete examples). Review source/test/dependency closures by behavior. External-editor terminal suspend/resume, native clipboard behavior, viewport resizing and visual fidelity were not exercised; LaTeX/math integration was only sampled and is not approved by this review. Session/tree/RPC changes are deliberately left to the other owner.

The parent's full gates may establish compilation/regression results for an exact candidate; they cannot alone establish pi conformance. This report does not reuse old audit test counts or claim new gate results.

## Suggested port-first order

1. **Freeze the implemented pi workflow matrix and inherited candidate boundaries.** Mark each operation implemented/partial/missing/accepted-divergence with its scenario tests. Fold in the other reviewer's session/RPC dependencies; no broad architecture cleanup first.
2. **Repair seams in existing TUI work:** attachment completion, clear/exit/remaps, typed image/text preservation, truthful settings/reload messaging (01, 04, the defect portion of 08 and 02). Characterize application-level behavior before integrating these dirty slices.
3. **Complete seven-tool contracts**, starting with real read pagination (05), preserving current mutation safeguards. Inventory schemas, output/truncation metadata and error behavior rather than counting tool names.
4. **Port local resource semantics** (03 then 02): YAML/discovery/collisions, prompt templates, source diagnostics, shared startup/reload and actual settings/scoped model controls. This is the foundation for extensions/packages.
5. **Add the missing subscription/provider slice** (07) with local fixtures and truthful picker availability. It can proceed alongside resource work once the parent's provider correctness prerequisites are clear; no live calls without separate approval.
6. **Port the isolated extension API and package lifecycle** (06, remaining 02), using the local resource model. Do not adopt unrelated plugin-system redesigns first.
7. **Finish multimodal and terminal parity** (remaining 08, themes/graphics/editor scenarios). Bounded image input precedes terminal graphics; explicit deferral is partial completion, not faithful-port completion.

**Residual limitation:** source-only findings and suggested tests; no runtime reproductions. Exact native-platform behavior, full provider inventory, package compatibility and the complete inherited visual diff remain to be validated. No blockers prevented writing this report; large diff outputs were sampled/targeted rather than treated as complete review evidence.
