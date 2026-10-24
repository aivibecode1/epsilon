# Status

**This is the living record.** It says what is true now, what changed, and what is
still limited. The permanent rules live in [`../AGENTS.md`](../AGENTS.md); the
decisions behind the product live in the
[ADR log](design/epsilon-decisions.md).

Last updated: 2026-10-24.

## Current state

Epsilon is a release candidate: one Rust binary with a terminal IDE over one agent core,
plus headless, HTTP/SSE and stdio-RPC modes. The release classification, verified
evidence and remaining caveats are in [`../RELEASE.md`](../RELEASE.md).

**Verified (2026-10-24):** the full battery — nine repository checks, `fmt`,
`clippy -D warnings`, the workspace tests, the offline demo and the real-feature
build — is **14/14 legs green**. Evidence:
`target/gate-final-audit-7.log`, run with
`CARGO_BUILD_JOBS=1`, `CARGO_PROFILE_DEV_DEBUG=0`,
`CARGO_PROFILE_TEST_DEBUG=0` and `CARGO_TARGET_DIR=%TEMP%\epsilon-target`.
The host's `stable` rustup toolchain and the VS2022 linker were incomplete, so
this run used the installed `1.97.1` toolchain and the Visual Studio 2019
Build Tools environment; that is an environment note, not a project pin.

The workspace suite is **77 test binaries, 2,658 passed / 0 failed / 21
ignored**; the terminal-IDE suite is **918 passed / 0 failed / 20 ignored**.
The numbers are summed from that log, not carried over from an earlier tree. The
first battery on this tree, `gate-goal-20261023-ui-skills-harness-1`, failed
`check-links` on five links inside the bundled `improve` skill; moving its assets
into `builtin_skills/improve/` made those links real relative links. The second
run was green before the footer leaf fix; the third before the browser. The fifth
failed one `epsilon-session` lease test under load; the same test passed alone
and the sixth run was green before the subagent rename. The seventh run, after
`/harness` became `/subagent`, was green before the final-review pass.
`final-review-1` caught a test-name deletion and `final-review-2` caught a parity
citation that the composer cleanup had shifted into a test module;
`final-review-3` was green before the README-in-skills-root fix, `final-review-4`
before the startup skill-summary change, and `final-review-5` before the card
pass. `cards-1` was the header-only version, `cards-3` the independent-card
version, and `final-audit-7` is the current record after the cache-tail and live-explorer fixes.

**The previous claim in this file was stale.** It read *"2,618 passed / 0 failed /
19 ignored"* and cited `audit/evidence/gate-f17b-20261022.log`. That count was
measured on a tree before the 2026-10-23 changes, and the cited log is not the
current record. The numbers above are the ones this tree produced.

**A real TUI defect was found by the full run.** The file editor drew its overlay
inside the workspace rect while the docked explorer stayed painted to its right,
so the screen was sliced beside the editor. The test
`the_file_editor_clears_the_screen_behind_it` failed deterministically. The
overlay now clears the full width between header and footer; that test passes, and
the frame dump at 100×30, 80×24 and 60×20 shows the editor as one panel with no
live screen beside it.

**Architecture.** The review that produced the current direction is
[`08-ARCHITECTURE-REVIEW.md`](08-ARCHITECTURE-REVIEW.md): a comparative study of the
six vendored harnesses and the REA component in `../references/`, the gaps it found
in this tree, and the work waves that follow from them. It is a plan, not a record of
finished work — [`08-ARCHITECTURE-REVIEW.md`](08-ARCHITECTURE-REVIEW.md) § "Waves" says
what is still to do.

## What changed on 2026-10-23

### Built-in skills are opt-in and behave like ordinary skills

- **Twelve curated skills now ship inside the binary**: `verify-before-claim`,
  `onboard-codebase`, `make-a-change`, `review-a-change`, `two-stage-review`,
  `ponytail`, `improve`, `ux-triage`, `write-a-commit`, `debug-a-failure`,
  `write-a-handoff` and `reverse-engineer-anything`. They are compiled from
  `crates/epsilon-core/src/builtin_skills/` and are **off until the user asks**:
  `/skills` lists what each does, why it is useful and its upstream credit, and
  `/skill <name> on` writes `<config>/builtin-skills/<name>/SKILL.md`.
- **A skill may own more than one file.** `improve` carries its three reference
  pages; enabling it writes them beside `SKILL.md`, disabling checks every owned
  file before deleting any, and a missing bundled file is repaired on the next
  enable.
- **There is one loader.** Once enabled, the file is discovered by the ordinary
  skill path — catalog, `skill` tool, scope labels, `/reload`. `/skill <name> off`
  removes a pristine copy and refuses to delete an edited one.
- **Ponytail and improve keep their MIT credit.** `BuiltinSkill.credit` carries
  “Ponytail by DietrichGebert · MIT” and “improve by shadcn · MIT” into
  `/skills`; their bodies are adapted upstream text, not Epsilon-authored prose.
- Docs: [`user/skills.md`](user/skills.md), [`design/epsilon-skills.md`](design/epsilon-skills.md),
  `/skills` and `/skill` rows in [`07-COMMANDS.md`](07-COMMANDS.md). ADR-083.

### The composer is a box, and answers carry a marker

- The prompt is now **one box**: the layout's existing gap row is the titled top
  border, the existing rule row is the bottom, and the side rails cost two
  columns rather than two rows. The border is the focus colour while the composer
  owns the keyboard and the quiet border otherwise.
- **Assistant prose always carries its own marker**, and continuations use the
  vertical rail. The user turn keeps its bar and prompt glyph. Colour and the
  background band reinforce the role; a 16-colour or `NO_COLOR` terminal still
  reads it.
- **There is no outer full-screen frame.** A second border around the same
  content would nest boxes; the composer is the control that gets the box,
  overlays draw their own, and the transcript stays a rail-based reading surface.
- The footer no longer prints `...\...\epsilon-tui` or cuts the leaf mid-word:
  a location that was already shortened is shortened once, and when even
  `...\epsilon-tui` will not fit, the whole leaf `epsilon-tui` is shown.
- Docs: [`09-TUI-REFERENCE.md`](09-TUI-REFERENCE.md). ADR-086.

### The transcript is card chat, not one text block

- Every user and assistant message is an independent four-sided card:
  `╭─ ▎ you ─...╮` / `╰──...╯`. There is no blank row between cards; the bottom
  border of one and the top border of the next are the separation, so each card
  costs two chrome rows rather than three.
- The card body wraps to the frame minus the two rails and their one-column
  padding. Tool cards keep their title and body rail; notices and reasoning stay
  ordinary rows.
- The live streaming answer is already a card while it streams and does not
  change shape on commit. At 80×24 the transcript shows roughly four to five
  messages; the owner accepted that density trade for the clearer boundaries.
- Docs: [`09-TUI-REFERENCE.md`](09-TUI-REFERENCE.md). ADR-091.

### Prompt cache follows the moving tool boundary

- The Anthropic tail breakpoint now lands on the last **user-role** turn, including
  tool results. Previously it stayed on the original user prompt, so every tool
  result after it was reprocessed at full price.
- Consecutive tool results are grouped into one user message with one
  `tool_result` block per call; the marker is on the last block. This matches
  Pi's `convertMessages` and removes separate consecutive user messages.
- The stable/system/tool breakpoints are unchanged; three of Anthropic's four are
  spent. Covered by `cache_breakpoints` under `--features real`. ADR-093.
- **OpenAI and Mistral now receive a prompt-cache key.** `LlmContext` has no
  session id, so the key is a stable FNV-1a hash of model + system prompt + first
  user text: the same conversation routes to the same prefix cache across tool
  round-trips and process restarts. It is not Pi's literal session-id key, but it
  is the same affinity mechanism without adding session state to the wire layer.
  Custom OpenAI-compatible endpoints and DeepSeek do not receive the field; the host match is exact, so a lookalike host cannot inherit an OpenAI-only parameter.

### The explorer is a live view

- A visible tree re-walks the workspace every 900 ms with a path/mtime/size
  stamp; the selected path survives a rebuild. `git status` refreshes every 3 s
  as the second signal, because index changes and commits leave file mtimes
  untouched.
- The status column is right-aligned: `A`, `M`, `?`, `D`, `R`, `!`; a changed
  directory gets a roll-up dot. Nesting uses `│` guides rather than blank
  indentation, and a folded directory can no longer hide a change.
- A hidden explorer costs nothing; opening or focusing it refreshes immediately.
  No filesystem-watcher dependency was added. `git status` resolves an absolute
  PATH binary and disables `core.fsmonitor`, so a workspace cannot plant the
  executable or run a hook when the tree polls. ADR-092.
- The `tree-status` frame fixture is at 100×30; the poll is covered by new-file,
  in-place-edit and no-change tests.

### `/skills` and `/subagent` are browsers, not transcript walls

- Bare `/skills` and `/subagent` open a **centred, read-only browser**: a windowed
  list with state, name and purpose, plus a detail block with the selected row's
  description, why, credit, scope and the exact writer command.
- `up`/`down`/`PageUp`/`PageDown`/`Home`/`End` move; `Enter` puts the selected
  writer command into the prompt, `Esc` closes. The commands remain the only
  writers, so the panel cannot drift from the installed state.
- The old transcript listing is gone. At 80×24 the panel is about twenty rows and
  the prompt box stays visible underneath. A blank row separates the built-in
  roles from discovered skills, so the two populations do not read as one list.
- Docs: [`09-TUI-REFERENCE.md`](09-TUI-REFERENCE.md), `/skills` and `/subagent` in
  [`07-COMMANDS.md`](07-COMMANDS.md). ADR-089.

### Four built-in subagent roles are opt-in

- `explore`, `review`, `verify` and `implement` are compiled into
  `crates/epsilon-core/src/builtin_subagents.rs`. `/subagent` opens the browser
  with what each does, why it is useful and `[on]`/`[off]`; `/subagent <name> on`
  writes an ordinary agent definition under `<config>/builtin-agents/`.
- **Naming follows DeepSeek Harness.** The delegated role is a **subagent**, not a
  "harness"; `/subagents` is the plural alias, and "harness" stays the whole
  program. ADR-090.
- **One loader again.** Discovery, the `task` tool, the tool subset and the
  parent's permission gate are the existing ones. A subagent role is absent until
  installed, so it does not change what the model may delegate by default.
- **The quoted-description bug is fixed.** Agent frontmatter now strips one
  matching pair of quotes from `name`/`description`, so a description containing
  `: ` arrives without literal quotes. This is not a YAML parser; it is the one
  scalar feature the bundle needs.
- Docs: [`user/subagents.md`](user/subagents.md), `/subagent` row in
  [`07-COMMANDS.md`](07-COMMANDS.md), [`10-EXTENSION-POINTS.md`](10-EXTENSION-POINTS.md).
  ADR-087 and ADR-088.

### REA is a first-class optional component

- `epsilon --rea` attaches the pinned `rea-agents@6.7.0` MCP server over stdio. The
  Windows `cmd.exe /d /c npx …` wrapper and a 900-second per-call deadline live in
  `epsilon-runtime/src/integrations.rs`; an explicit `rea` MCP entry wins, so the
  flag never registers a duplicate.
- `[[mcp_servers]]` and the HTTP form accept `timeout_secs` (default 120, valid
  1–86,400). The deadline is per server, so a slow analysis integration does not
  weaken every other MCP call.
- The `reverse-engineer-anything` built-in skill carries the workflow and evidence
  rules. Credit: REA by morluto, MIT,
  <https://github.com/morluto/rea>; see [`references/rea.md`](references/rea.md) and
  [`user/reverse-engineering.md`](user/reverse-engineering.md). ADR-084.
- **Live attach verified on this machine:** with Node 24.18.1,
  `epsilon --rea --provider faux -p hello` printed
  `mcp server 'rea' attached (207 tools)` and completed the faux turn. The first run
  downloaded the pinned package through `npx`.

### Final review pass

- **Architecture and references.** The delegation path is still one
  `epsilon-runtime::build_runtime` composition into one `epsilon-core::Agent`;
  the TUI, headless, HTTP and RPC modes remain reducers over the same events. The
  DeepSeek Harness source under `references/deepseek-harness-dsh-v0.2.1-rc.2`
  confirmed its model-facing tool is `subagent`; Epsilon adopted that vocabulary
  (`/subagent`, `builtin_subagents`) in ADR-090 but kept its own `task` tool and
  agent-definition loader. Pi's filled user/tool blocks and one-rule composer were
  checked against the actual frames; Epsilon keeps the filled bands and now one
  composer box.
- **TUI.** The subagent browser now sizes to its content instead of a fixed 20
  rows (the 80×24 frame had seven empty rows inside the box), its hint fits
  without clipping, and the footer no longer prints a leading path separator.
  The unused pre-box `draw_rule` was deleted; the old rule test was replaced with
  `the_rule_fills_its_row_exactly`, which asserts the composer box's bottom
  border, so coverage is not silently lost.
- **Installers.** `install.sh --help` previously cut its own paragraph at
  "some"; the usage range now includes the whole sentence. `bash -n`, `--help`,
  an unknown option and a missing `--prefix` value were exercised. The Windows
  installer was run end-to-end, including `-Prefix` into a temporary root; this
  WSL has no Linux Rust toolchain, so the Linux/macOS `cargo install` path is
  syntax- and argument-checked, not executed. The status does not claim a macOS
  or Linux build.
- **Skills and REA.** The built-in skill tests cover install, discovery, opt-in
  state, bundled reference files, refusal to overwrite edits and the 12 built-in
  skills. A flat skills root now ignores `README.md`: a documentation file there
  was announced as an invalid skill on every launch. Startup now reports
  `skills: N loaded` instead of printing every discovered name in one unbounded
  line; the `/skills` browser is where the list belongs. REA remains optional and
  was re-attached successfully after the final install, registering 207 tools.

### Installation and documentation

- **`install.ps1` and `install.sh`** are the supported source installers: Rust
  version check, `cargo install --locked --force`, out-of-tree target directory,
  post-install `epsilon --version`, `--prefix`/`-Prefix` and `--no-verify`. ADR-085.
- New references: [`09-TUI-REFERENCE.md`](09-TUI-REFERENCE.md) and
  [`10-EXTENSION-POINTS.md`](10-EXTENSION-POINTS.md). The docs map, README, reference
  index and credits now include both REA and the installers.
- The `epsilon-safety` test-only `unused_mut` warning that made `clippy -D warnings`
  fail on 2026-10-22 was removed.
- **The project tree no longer carries legacy archives.** `audit/`, `agents/memory/`,
  `docs/milestones/`, `docs/plans/` and the hidden `.pi/` state were removed on
  2026-10-23. The current record is the numbered docs, `design/`, `user/`, `parity/`
  and `references/`; the link checker is clean at **289 relative links, 0 broken**.
- **The file editor now clears the whole width behind it.** The overlay used the
  workspace rect, so a docked explorer stayed painted to its right and the screen
  was sliced beside the panel. It now clears the full band between header and
  footer; the existing deterministic test passes, and the 100×30 / 80×24 / 60×20
  frame dump shows one panel with no live screen beside it.

### Verification

- **Battery:** 14/14 legs green,
  `target/gate-final-audit-7.log`, debug info off, one build
  job, isolated target.
- **Tests:** 77 binaries, 2,658 passed / 0 failed / 21 ignored; terminal suite
  918 passed / 0 failed / 20 ignored.
- **Cache wire:** `cargo test -p epsilon-llm --features real --test cache_breakpoints`
  passed 7/7, including the tool-result tail marker, parallel-result grouping and
  the forward-moving boundary across tool round-trips.
- **REA:** live `npx` attach registered 207 tools (Node 24.18.1).
- **Installer:** `install.ps1` built with `cargo install --locked --force` and
  installed `epsilon 0.1.0` at `C:\Users\Hp\.cargo\bin\epsilon.exe` (2026-10-24
  17:04 local, after cache-tail, live-explorer, git-hardening and exact-host OpenAI/Mistral cache-key fixes). The host's `stable` rustup
  toolchain and VS2022 linker were incomplete, so the build used the installed
  `1.97.1` toolchain and Visual Studio 2019 Build Tools; that is an environment
  workaround, not a project pin. The installed binary answered `--version`,
  completed `--demo` offline, and attached REA with 207 tools. The build warned
  that `clipboard-win v5.4.1` is yanked in the registry but `--locked` still
  produced the binary. `install.sh` was syntax-checked with `bash -n` on this
  host; it was not executed on macOS or Linux here, and the status does not
  claim it was.
- **Frame dump:** the ignored `frame_dump` tests render the layout scenarios at
  100×30, 80×24 and 60×20, including the explorer with `M`/`?` markers and a
  changed-directory dot; the editor, palette, search and streaming frames were
  read, not only asserted on state.

## What changed on 2026-10-22

### Defects fixed and verified

- **A refused exit is now visible.** `Ctrl+Q` during a run or with unsaved edits was
  refused, but the refusal was only a transcript line — and the file viewer is a
  panel over the transcript, so a reader with a dirty tab saw nothing and pressed
  `Ctrl+Q` again. The footer's mode word now reads `quit refused`, *additively*: the
  surface's own hints still print, because the viewer's `alt+s save` is the key the
  reader needs next. `crates/epsilon-tui/src/app/commands.rs`,
  `crates/epsilon-tui/src/app/view.rs`, and a frame test in
  `crates/epsilon-tui/src/app/defect_tests.rs`.
- **The quit path had two implementations.** `Command::Quit` called one exit routine
  and the keybinding called another; the first was dead code that `clippy -D warnings`
  rejected. Both now go through `App::request_quit`, which refuses on an in-flight
  run or a dirty viewer, says why, and clears the refusal on a successful exit.
- **The settings writer's staging name was unique per process, not per write.**
  `write_config_atomic` named its staging file `<file>.toml.tmp.<pid>`, so two
  threads in one process writing the same file shared one staging path: the first
  renamed it into place and the second's rename failed with `os error 2`. The prompt
  history is written from the TUI's submit path, so **two submits racing in one
  session lost one of them**. Fixed with a monotonic per-process counter;
  `crates/epsilon-core/src/config.rs` now has a test that asserts the invariant that
  causes the failure — two concurrent writes use two different staging paths — and
  that test was verified to fail against the old naming and pass against the new one.
  **Verified RED → GREEN**, both quoted in
  [`08-ARCHITECTURE-REVIEW.md`](08-ARCHITECTURE-REVIEW.md) (F25):

  ```
  # RED — the old naming
  assertion `left != right` failed: two writes of one file must not share a staging
  path: "…\history.toml.tmp.26984" == "…\history.toml.tmp.26984"
  test result: FAILED. 1 passed; 1 failed

  # GREEN — the counter
  test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 262 filtered out
  ```
- **Two parity rows claimed Epsilon lacks a session delete it has.** P-070 and P-236
  in `docs/parity/pi-tui.md` said the session browser had no cross-directory scope
  and no delete; it has both (Tab switches the scope, Ctrl+D deletes with a
  confirmation and an active-session refusal). The rows and the area summary that
  contradicted them are rewritten.
- **`02-ARCHITECTURE.md` described a manifest edge that no longer exists** — the
  `epsilon-cli` → `epsilon-ext` edge. It was removed; the document still named it as
  present.
- **`TUI-DECISION.md` understated the switching cost by 1.75×** and cited the
  `epsilon-desktop` crate deleted in ADR-039. Re-measured on this tree: **66,482
  lines / 57 files / 689 tests**, against the document's 38,067 / 49 / 509. The
  conclusion survives; the argument is now made from a current measurement.

### Known limits, measured

- The parity matrix in `docs/parity/pi-tui.md` claims "48 offered items"; the source
  has 43. It also cites two tests that do not exist in `crates/`
  (`compact_from_the_typed_line_compacts_and_reports`, `slash_import_switches_live`).
- **The rule-matching loop existed in three copies.** The gate hand-rolled it
  twice — once for the plan layer, once for the preset layer — beside the engine's
  own copy. The gate now composes **plan + base + preset** into one
  `PermissionEngine` and evaluates it once; `glob_match` and the hand-rolled
  capability matching no longer appear in `gate.rs` at all. The matching now lives
  in `Rule::matches` (`crates/epsilon-safety/src/engine.rs:42`) and is called from
  exactly one place.
  The divergence it removes was **latent, not live**: the engine lets a layer's
  *allow* win, while the gate's plan loop honoured only *denies*, so a plan-layer
  allow was inert in the gate and would have won in the engine. `plan_ruleset()` has
  only denies today, so nothing had broken — but it was a trap for whoever added an
  allow next. The invariant is now tested:
  `the_gate_and_the_composed_engine_agree` walks every posture the gate can be in
  against eight capability/subject pairs and asserts the two agree.
- **Eleven registered tools no longer ask for permission in a default session.** The
  default ruleset named nine capabilities while the registry registers eighteen, so
  `find` (a *read*), `todo`, `task`, `skill`, `lsp`, `question`, the three goal tools
  and `apply_patch` fell through to `Ask` — a dialog for every one, in a session that
  had granted nothing. Three changes, in `crates/epsilon-safety/src/engine.rs`,
  `crates/epsilon-contracts/src/tool.rs`, `crates/epsilon-tools/src/apply_patch.rs`
  and `crates/epsilon-core/src/exec.rs`:
  - The free set is now **derived** from one classification
    (`READ_ONLY_CAPABILITIES`) instead of a second hand-written list, so the two
    cannot drift apart again.
  - A tool declares the **capability** it acts under (`Tool::capability()`, default
    the tool's own name). `apply_patch` declares `edit`, so a rule the user wrote
    for `edit` covers it.
  - A test reads the **live registry** and asserts that the only tools that ask are
    the ones that ask by decision — `bash` and `webfetch`, the two that reach
    outside the workspace. A new tool that lands and asks has to say why.

  This is the highest-ranked defect in
  [`08-ARCHITECTURE-REVIEW.md`](08-ARCHITECTURE-REVIEW.md) (F17), and it was
  security-adjacent: a user who meets a dialog for everything learns to approve
  everything.
  **Proven end to end**: `a_read_only_tool_runs_without_a_dialog_and_the_shell_still_asks`
  drives a real `find` call through the real loop with a UI that denies every ask —
  the call succeeds, so it did not ask — and then the same test shows `bash` asking
  exactly once. One test, both halves, so a change that made everything ask would
  fail it too.
- A subagent's result body has no size cap (F26), and a failed tool call is repaired
  only when the session is reopened (F27) — so a run whose tool failed mid-call
  carries a transcript the next provider request will reject. Both are in Wave 3a
  of the review and neither is fixed.
- The permission policy is never told to the model (F28): a refused call arrives as a
  tool error, so the model learns the rule one violation at a time.

## What changed on 2026-10-18

### Interface

- **The slash menu and the command palette are the composer's width.** Both used to be
  narrower than the input they belong to (the popup capped at 80 columns, the palette at
  80% of the frame), so descriptions ran out of room while the frame kept empty page.
  They now share the workspace column, start on the same edge, and sit above the
  composer.
- **Rows are columns, not a sentence.** Each row has a measured command column, a
  description column that starts at one x position, and a right-aligned key hint. The
  `command · description` join and its clipped ellipses are gone. The shared helper is
  `ui::overlay::list_row_columns`.
- **Descriptions are one imperative line.** All 43 offered rows were rewritten: for
  example, `/search` says "Search the transcript" (not "this session"; it does not read
  the session file), `/scoped_models` says "Choose models for F2 cycling" (not Ctrl+P,
  which opens the palette), and `/reload` no longer claims to reload configuration it
  does not re-read. The full usage of every command is in
  [`07-COMMANDS.md`](07-COMMANDS.md).

### Documentation

- The numbered sequence is now `01-OVERVIEW` → `07-COMMANDS`, followed by the detailed
  subject folders. The archive, milestone logs, old plans, agent memory and audit
  markdown were removed; there is no second, historical copy of the docs to drift.
- [`07-COMMANDS.md`](07-COMMANDS.md) documents every command, alias, confirmation and
  mode boundary in one place.
- The reference studies were re-derived against the vendored Pi 1.3.0 and DeepSeek
  Harness 0.2.1-rc.2 trees, and the old `pi-1.0.0` / `0.2.1-alpha.1` citations were
  re-pointed.

### Security

A read-only review of Pi 1.3.0 and DSH 0.2.1-rc.2 against this tree found twelve
defects worth fixing. All are closed; where the failure is observable, a test or a frame
fails for it.

1. **"Always (session)" remembered a shell command's first word.** The dialog showed
   `git status; curl … | sh`; the gate remembered `git`, so a later compound command
   with the same head ran unprompted. The subject is now the whole command line
   (`epsilon-core/src/exec.rs`), and session memory compares exactly, not by glob
   (`epsilon-safety/src/gate.rs`). Config rules keep their globs.
2. **Plan mode and `read-only` denied only four tool names.** An MCP or extension write
   matched no rule, fell through to `Ask`, and a user could approve it from a posture
   that promises the tree cannot change. Both postures now use a read-only capability
   allowlist and deny everything else before the base layer
   (`engine::is_read_only_capability`).
3. **The serve bearer token was a hash of time, pid and a counter.** It now comes from
   the OS CSPRNG, and an entropy failure refuses to start the server
   (`epsilon-server/src/lib.rs`).
4. **`webfetch` followed redirects and could reach loopback, private or metadata
   addresses.** The client no longer follows redirects, resolves the host once, rejects
   non-public addresses, pins the checked address, and streams the body into its cap
   instead of buffering first (`epsilon-tools/src/webfetch.rs`).
5. **An unconfined `bash` timeout or cancel killed only the direct child.** Descendants
   now die with the command: a process group on Unix, `taskkill /T` on Windows
   (`epsilon-tools/src/shell.rs`).
6. **Session files were read whole with no bound.** The store and pi-import refuse a
   file over 256 MiB, and discovery reads at most 1 MiB of a header or first message
   (`epsilon-session`).
7. **A checkpoint ledger could point outside the workspace.** `/changes` now drops rows
   whose path does not resolve inside the workspace; `/restore` already refused them.
8. **Windows ran a workspace-planted `powershell.exe`.** The shell was spawned by bare
   name, and `CreateProcess` searches the parent's current directory before PATH. The
   interpreter is resolved to an absolute `pwsh.exe`/`powershell.exe` path and the spawn
   fails rather than falling back to a search.
9. **Goal gates bypassed the session policy and hardcoded workspace-write.** A
   model-written gate called the approval UI directly, so plan and `read-only` never saw
   it, and it ran with write access under `--sandbox read-only`. The runner now carries
   the session `PermissionGate` and confinement mode; a deny stops the gate before it
   runs, and the tool and front end share one runner.

Three more findings came from the same review and are also closed:

- **`--sandbox read-only` was narrower than its name.** It set the shell mode but left
  `write`/`edit` allowed, and on Windows the Job Object cannot enforce a filesystem
  boundary at all. The launch now forces the `read-only` permission preset too, and the
  shell refuses a read-only request when the backend reports no filesystem boundary.
- **Project instructions were read without trust, cap or escaping.** `AGENTS.md` and
  `MEMORY.md` entered the system prompt even when the user declined trust; one file
  could close the `<project_instructions>` frame. The agent now takes the frontend's
  trust decision, each file is capped at 256 KiB with a marker, and the closing tag is
  escaped.
- **Headless `-p` printed model output raw.** The final text and the diagnostic now go
  through the control/bidi stripper before they reach the terminal.

Also fixed: `/export jsonl` writes JSONL rather than pretty JSON under a `.jsonl` name.

## What changed on 2026-10-19

### TUI IDE reliability

- **Dirty file edits remain visible and recoverable.** A draft stays visible in
  view mode, `e` resumes it, and the bound save key works in either mode. Ctrl+Q
  and `/quit` refuse while any open tab has unsaved changes; closing a dirty tab
  remains an explicit discard. See [ADR-080](design/epsilon-decisions.md#L5037-L5069).
- **Esc-abort preserves queued work and the composer draft.** Unconsumed steering
  and follow-up messages remain as their original `Message` values, so an
  `@`-reference snapshot is not flattened or lost. Steering follows the next
  run's normal drain; Alt+Q remains the explicit steering-to-editor path. See
  [ADR-081](design/epsilon-decisions.md#L5071-L5108).
- **File previews are bounded.** The TUI reads at most 256 KiB plus one probe
  byte and keeps at most 2,000 lines. Partial previews are marked and read-only;
  later-window paging remains out of scope. See
  [ADR-082](design/epsilon-decisions.md#L5110-L5141).

### Reference analysis

Pi's two-level loop and queue model are already present in Epsilon. Pi's
interactive abort behavior was a useful comparison for recovering queued input.
DSH's workspace-file service supplied the bounded-read pattern; its plugin
kernel and Web/Electron carrier remain different product choices. The reference
studies are updated in [`references/pi.md`](references/pi.md) and
[`references/dsh.md`](references/dsh.md).

## Known limitations

Honest limits, not a roadmap:

- **Credentials are plaintext** in `<config dir>/auth.json` (0600 on Unix, atomic
  rename, a warning at login/list). This follows Pi 1.3.0's own `auth.json`
  model; the owner accepted it, and an OS keyring is optional hardening rather
  than a parity requirement.
- **Windows confinement is partial.** Job Objects bound memory and the process tree, not
  the filesystem or network. `--sandbox read-only` refuses a shell command on Windows
  rather than pretending; use `/plan` for a read-only session.
- **Session transcripts live in the workspace** under `.epsilon/sessions/`, now in
  owner-only files and per-launch names. A repository that ships a session file is still
  readable input to `-c`; review a clone before continuing a session in it.
- **Image reads decode in-process.** The input byte cap bounds how much compressed data
  is read; the `image` crate's allocation limits are the only bound on a decompression
  bomb. Treat an untrusted image like any untrusted binary.
- **The terminal file viewer shows a bounded prefix** (2,000 lines or 256 KiB)
  and does not page to later ranges; open larger files in an external editor.
- **Live-only model ids** are visible but not routable; they show `[unavailable]` rather
  than guessing context, price or capabilities.
- **No screen-reader pass** has been recorded for the terminal IDE. Contrast and
  keyboard reachability are tested; an announcement pass is human work.
- **MCP and extension tools are denied in plan mode** unless they are one of the
  read-only capabilities. That is deliberate: a third-party tool's intent is not
  knowable from its name.
- **`/share` needs `gh`** installed and authenticated; without it the command writes the
  HTML export and prints the gist command.
- **Built-in skills are opt-in and cost context once enabled.** The twelve entries are
  absent from the prompt until `/skill <name> on`; after that each one adds its name and
  description to the catalogue and can be loaded into the conversation. They are
  procedures, not new capabilities.
- **REA needs Node 22.19+/24.11+ and network on first launch**, and native analysis needs
  an installed Hopper, Ghidra or IDA. A live attach was verified here on Node 24.18.1:
  `epsilon --rea --provider faux -p hello` started `rea-agents@6.7.0` over stdio and
  registered **207 MCP tools**. The MCP server runs outside the sandbox, so point it only
  at the target.

## How to verify

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -q -p epsilon-cli -- --demo
cargo check -p epsilon-cli --features real
```

The full battery is the same list plus the repository checks:

```sh
pwsh -File scripts/gate.ps1 <label>   # Windows
bash scripts/gate.sh <label>          # POSIX / CI
```

The supported install path is the checked installer:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

```sh
bash install.sh
```

Both use `cargo install --locked --force` and verify `epsilon --version` unless
`--no-verify` is passed.

See [`03-DEVELOPMENT.md`](03-DEVELOPMENT.md) for the machine rules, the log locations
and the recovery procedures.

## Next work

The security review left no unreviewed high-severity finding. Credential storage
is **Pi-parity** (`auth.json`, mode 0600, plaintext); an OS-keyring backend is
optional hardening, not a release blocker. The remaining platform work is the
missing Windows read-only backend, a human accessibility pass and the unrun
macOS installer job. Each needs an ADR before it changes behaviour; none is a
quiet follow-up.

The skills/subagent browser covers the long-list clutter, and user and assistant
messages are now independent cards. The accepted cost is density: at 80×24 the
transcript shows roughly four to five messages. Tool calls keep their existing
title-and-rail cards; boxing them separately, or grouping a turn's tool calls
into the answer card, remains a later design decision with its own budget.

Three verification gaps are honest rather than hidden: `install.sh` has not been
executed on a macOS or Linux host from this Windows machine (a macOS CI job now
builds the workspace and runs the installer, but that job has not run yet); REA's
native engines (Hopper, Ghidra, IDA) were not present here — its MCP attach and
tool catalogue were verified, not a native decompilation; and the prompt-cache
fix is wire-verified rather than measured against a live paid provider.
