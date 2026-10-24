# AGENTS.md — Working in the Epsilon repository

**Version 6.0 · 2026-10-24**

This file is the repository's permanent engineering contract. It should be true
for the current tree, not for a past milestone. Progress, open defects and
environment notes live in [`docs/04-STATUS.md`](docs/04-STATUS.md); design
decisions live in [`docs/design/epsilon-decisions.md`](docs/design/epsilon-decisions.md).
When this file and the code disagree, inspect the code, fix the one that is wrong,
and record the change.

---

## 1. What Epsilon is

Epsilon is an agent harness with **one core and one terminal IDE**:

- `epsilon-cli` dispatches all modes; `epsilon-tui` is the full-screen terminal
  IDE: transcript, composer, file tree, viewer/editor, command palette, pickers.
- The same binary also runs headless (`-p`), HTTP/SSE (`serve`) and stdio-RPC.
- The agent loop is `epsilon-core::Agent`. Every frontend is a mode on that one
  loop, not a reimplementation.

The project began as a port of [Pi](https://github.com/earendil-works/pi) and has
taken ideas from DeepSeek Harness, Hermes Agent, opencode and codewhale-ratatui.
REA is an optional external MCP server for reverse engineering. Credits and
licences are in [`README.md`](README.md#credits) and [`docs/references/`](docs/references/README.md).

Current product facts (verify against source before changing them):

- **User and assistant messages are independent card boxes** in the transcript.
  Tool cards keep their own title/body treatment; notices stay ordinary rows.
- **The explorer is live** while visible: a workspace stamp poll, a `git status`
  poll, a persistent selection, a status column and indent guides. It is not a
  startup snapshot.
- **Four built-in subagent roles** are opt-in: `explore`, `review`, `verify`,
  `implement`. The command is `/subagent` (`/subagents` is an alias); a delegated
  role is a **subagent**, never a "harness".
- **Twelve built-in skills** are opt-in. `/skills` browses them; `/skill <name> on`
  writes an ordinary `SKILL.md`. REA is a separate optional MCP component.
- **No desktop frontend.** It was removed in ADR-039. A return needs a new ADR and
  a new battery shape, not a quiet re-add.

## 2. Architecture invariants

These change only with an ADR in
[`docs/design/epsilon-decisions.md`](docs/design/epsilon-decisions.md).

1. **One core.** `epsilon-core::Agent` owns the loop, scheduling, context assembly
   and compaction. A capability one mode has must not be reimplemented in another.
2. **One composition point.** `epsilon-runtime::build_runtime` / `resolve_launch`
   is the only place a workspace plus config becomes a running agent. A second
   composition path is a defect.
3. **No desktop frontend.** ADR-039 removed the egui/eframe crate and its pins. The
   retained `WINDOW` keybinding table belongs to the TUI IDE, not a second window.
4. **One command registry.** Slash commands (`COMMAND_HEADS`), palette entries and
   keybindings are declared once in `epsilon-contracts` and read by every mode,
   with per-mode capability declared as data, not a second hand-maintained list.
5. **No library depends on `epsilon-cli`.** Cargo prevents cycles, not accidental
   architecture edges. `epsilon-cli` may depend on the others; the reverse is a
   review item.
6. **A refusal path never exits the process.** Shared functions return an error;
   they do not `die()`. `resolve_launch` returns `Err(String)` because a refusal
   must reach the caller, not terminate it.
7. **Nothing a UI advertises may be a no-op.** A bound key the UI does not act on
   must say so; a palette entry must name a command that exists and works. A silent
   no-op is worse than a missing feature because it lies.
8. **Wire types are contracts.** A wire name or body shape is verified by sending
   or building a real frame, never by asserting serde's output against a variant
   list. Cache-control placement is a wire contract too (ADR-093).

## 3. Repository map

```
crates/
  epsilon-contracts   shared types and traits; zero internal dependencies
  epsilon-llm         provider protocols, faux provider, routing, catalog, auth, retry, cache policy
  epsilon-session     JSONL storage, branching, leases, repair
  epsilon-core        agent loop, scheduling, context assembly, compaction, tasks
  epsilon-tools       native tools, shell adapter, checkpoints, goals
  epsilon-safety      permission engine, approvals, sandbox backends
  epsilon-lsp         optional language-server client and diagnostics
  epsilon-ext         process-extension protocol and host
  epsilon-mcp         MCP client and tool adapter
  epsilon-runtime     COMPOSITION: config + workspace -> running agent
  epsilon-tui         terminal IDE: cards, tree, viewer/editor, palette (ratatui + crossterm)
  epsilon-server      HTTP/SSE frontend
  epsilon-cli         binary: argument parsing and mode dispatch
docs/                 01-OVERVIEW.md ... 10-EXTENSION-POINTS.md, design/, user/, parity/, references/
scripts/              gate.ps1 / gate.sh, check-*.py, console-probe.py
install.sh            macOS/Linux source installer
install.ps1           Windows source installer
references/           read-only vendored reference harnesses
```

`references/` is **input**: never modify, move or delete it. It is not part of
the build and is not committed here.

## 4. Build, test, run

Toolchain: **Rust 1.88+**, edition 2024, inherited uniformly from the workspace.
Do not raise the floor in one crate or pin a local toolchain in the repository.

```sh
cargo check --workspace --all-targets
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p epsilon-llm --features real   # provider wire tests, including cache breakpoints
cargo run -q -p epsilon-cli -- --demo       # offline scripted smoke test
cargo run -q -p epsilon-cli                # the terminal IDE
```

**The battery** runs the repository gates plus the workspace checks:

```sh
pwsh -File scripts/gate.ps1 <label>    # Windows; use this host
bash scripts/gate.sh <label>           # POSIX / CI
pwsh -File scripts/gate-until-pass.ps1 -Label <label>   # retry, stops on a repeat fingerprint
```

Fourteen legs, in order: `check-links`, `check-lint-tables`,
`check-parity-citations`, `check-citation-placement`, `check-reference-citations`,
`check-verb-counts`, `check-parity-counts`, `check-line-endings`,
`check-control-bytes`, `fmt`, `clippy`, `test`, `demo`, `real-check`. The log is
`target/gate-<label>.log`. Give it a timeout above 600 s; the test leg is the
long one. Copy evidence out of `target/`, which is disposable.

**Never reuse a battery label.** `gate.ps1` truncates the log; reusing one destroys
the failed attempt. **Never edit source while a battery runs** — the log would
describe a tree that no longer exists.

### This machine

- **One build owner.** A battery, a delegated build and `cargo install` must not
  run together. Use `CARGO_BUILD_JOBS=1`; parallel jobs have failed here with
  `0xc0000142` compiler-startup contention.
- **Build outside the checkout.** `CARGO_TARGET_DIR=%TEMP%\epsilon-target` is the
  default in `gate.ps1`; writing build output inside the repository has caused
  `cl.exe` `D8050: cannot execute c1.dll`. Export the target dir and
  `CARGO_BUILD_JOBS=1` before any shell build.
- **Debug info off for the battery** (`CARGO_PROFILE_DEV_DEBUG=0`,
  `CARGO_PROFILE_TEST_DEBUG=0`): roughly 4 GB instead of 33 GB. Name the knob
  when citing evidence. Turn it on only for a debugging round.
- **MSVC environment.** `gate.ps1` loads `vcvars64.bat` when `INCLUDE` is empty.
  If the host's VS2022 installation is incomplete, repair the host or use another
  installed MSVC environment; do not change the project's toolchain pin to mask a
  broken linker.
- **`LNK1104` on a freshly linked executable is a host fault, not a defect.**
  Avira can hold a path. Never change security configuration from an agent
  session; rebuild with fresh metadata and disclose the workaround.
- `bash` here is WSL and may lack a Windows `cargo`. Prefer `gate.ps1`; a POSIX
  host uses `gate.sh`.
- The repository is LF (`* text=auto eol=lf`).

## 5. Engineering rules

- **Ask Ponytail first.** Does the feature need to exist? Is it already here? Can
  the standard library or platform do it? Is an existing dependency enough? Then
  write the minimum. Mark a deliberate shortcut with a `ponytail:` comment naming
  its ceiling and upgrade trigger. Never let minimalism block a justified
  architectural fix, a trust-boundary check, accessibility, or data-loss
  protection.
- **Reuse before adding.** A helper or decision already implemented three files
  over is the common failure. If two modes need the same decision, put it in the
  core, not in both.
- **Smallest correct change.** Deletion beats addition. A refactor that also
  changes behaviour is not a refactor and needs its own test/ADR trail.
- **Rust:** `unsafe_code = "forbid"` workspace-wide; `missing_docs` is on. Direct
  dependencies are pinned exact (`=x.y.z`) in `[workspace.dependencies]`; no new
  dependency without a stated reason. New crates need an ADR first.
- **Comments explain why**, not what. Cite a reference with a real `file:line`.
- **Behaviour-level changes need an ADR entry or amendment.**
- **Settings are user work.** A read-modify-write must refuse a present but
  unparsable file and publish with staged write + rename. Use
  `epsilon_core::config::read_config_table` and `write_config_atomic`; do not add a
  bare `fs::write` settings writer.
- **No unmeasured performance claims.** Startup, frame cost, tree-poll cost and
  cache behaviour are numbers to measure, not adjectives.

### Evidence discipline

These failure modes have each cost real time. They share one shape: something
reports success while the thing the user wanted did not happen.

- **A passing test is not a true claim.** For anything a user sees, drive the real
  draw path and assert on the frame. Asserting on state has shipped unrendered
  features here.
- **Grep for callers and check one is production code.** A definition plus a test
  is not a feature.
- **A passing control is not a proven control.** Ask what the broken version would
  print and confirm it differs.
- **Measure the number the code reads**, not the one you reasoned about.
- **Check whether the frame is the bug** against the reference, not against your
  own naming.
- **Never quote a test count you did not measure.**
- **A frame is not asserted until you have looked at it.** Render and read it at
  80×24 and 100×30.

## 6. Security and data loss

- Every side-effecting tool call passes the permission gate. Deny is monotonic:
  session memory must never resurrect a denial. An unknown capability is `Ask`,
  never `Allow`.
- The approval channel **fails closed**; a dropped responder is a denial.
- Untrusted text (model output, tool output, file contents, session names) is
  stripped of control and bidi characters before it reaches a screen.
- **Credential storage follows Pi.** Provider credentials live in plaintext at
  `<config dir>/auth.json` (`crates/epsilon-llm/src/auth.rs`): mode 0600 on Unix,
  0700 directory, atomic rename, warning at login/list. Pi 1.3.0 ships the same
  `auth.json` model with mode 0600; an OS keyring is not part of Pi's credential
  path. The owner accepted this trade-off on 2026-10-17. Treat plaintext storage
  as an intentional reference-parity limitation, not a missing port; the
  OS-keyring backend is optional hardening that would need its own ADR, migration
  and tests. Do not add another plaintext secret format without a new owner
  decision. Release notes must name this limitation; [`RELEASE.md`](RELEASE.md)
  is the current record.
- **Never lose a user's work.** Confirm before discarding or overwriting; keep a
  recoverable copy; a refusal leaves the file untouched.
- Validate every boundary — paths, RPC frames, HTTP bodies, MCP payloads. A path
  crossing into the filesystem is normalised and checked for escape, not joined
  and trusted.
- Minimal dependencies. A new dependency is a supply-chain decision.

## 7. Testing

- **Faux provider first.** Full-loop scenarios run offline against scripted steps.
  Real-provider tests are gated behind features/env vars; nothing hits a paid API
  by default.
- Deterministic time. Use the project's slow provider; never sleep and hope.
- Each crate owns integration tests for its failure paths, not only the happy path.
- A test is not done until it fails for the reason it names. Prove causation when
  fixing a defect.
- **Wire and cache tests belong with the wire.** `epsilon-llm --features real`
  builds the actual request body; do not replace a wire test with a boolean-policy
  assertion.

## 8. UI and accessibility standard

Applies to the terminal IDE first, and to every other mode as far as each can.

- **WCAG 2.2 AA is the floor**, not a stretch goal.
- **Full keyboard operation**, logical focus order, visible focus, and no keyboard
  traps. Every mouse-reachable control is keyboard-reachable and operable.
- **The terminal cannot create platform accessibility nodes.** Epsilon is a
  painter-drawn TUI; do not claim platform screen-reader metadata it does not
  emit. Keep all state readable as terminal text, and never rely on colour alone.
- **Default layout is an IDE.** The explorer is docked right from the first frame
  (24–38 columns, 30% preferred, one-column gutter). Below the 44-column workspace
  floor it becomes a focused band between header and footer; it never squeezes the
  transcript.
- **Visibility and focus are separate.** `tree_open` paints; `tree_focused` owns
  the keyboard. `F6` focuses a resting explorer and hides a focused one; `Esc`
  returns focus without closing it.
- **Contrast:** body ≥ 4.5:1, secondary text and boundaries ≥ 3:1, in every theme.
- **Every async view has states:** loading, empty, error, cancelled, long-running.
  No silent failure; no data loss; recovery and confirmation where required.
- **Performance:** fast startup, virtualised long transcripts and files, no
  blocking of the UI thread. The tree poll is bounded; do not turn it into an
  unbounded per-frame walk.

## 9. Documentation

- [`docs/01-OVERVIEW.md`](docs/01-OVERVIEW.md) is the newcomer reading guide. Keep
  it true.
- **Document intent, structure and decisions**, not what the code already says.
- Plain and concrete. Short sentences. Define a term at first use. No marketing.
- **Every command in the docs must have been run. Every link must resolve.**
  `scripts/check-links.py` is a gate.
- New documentation files go in [`docs/README.md`](docs/README.md), the map.
- `docs/04-STATUS.md` is the moving record: current evidence, known limits and
  what changed. Do not let it become a changelog of unrelated milestones.

## 10. Credits and licences

- **Attribution is mandatory and respectful.** Take authors, links and licences
  from the project's own README, LICENSE and package metadata — never memory.
- Never belittle a reference. Describe differences in goals and design.
- **Ideas versus code.** Check licence compatibility before adapting. Compatible:
  keep notices in `THIRD_PARTY_NOTICES` and mark adapted files. Unclear:
  reimplement the idea. Never present others' work as ours or take a name, logo,
  mascot or brand.
- `references/` is read-only input.

## 11. Workspace hygiene

- Remove caches, temporary files and generated output when they are useless. Keep
  the source and the build the project needs; no parallel experimental builds.
- Before deleting, check `git status` that the target is untracked and
  regenerable. **Never run a blanket clean** (`git clean -fdx`, broad `rm -rf`).
- Protect `references/`, `.git`, `AGENTS.md`, lockfiles and user data.
- Do not push, publish or touch a remote from an agent session. Never commit
  secrets.

## 12. Subagents

Every brief begins: **read and obey `AGENTS.md`**. Subagents never edit
`AGENTS.md`; this file is owned by the integrating agent.

Give each subagent a reason to exist, a narrow brief, explicit file ownership
(never two writers on one file), a defined output, and the checks it must run.
Scale parallelism to the machine; one build owner at a time.

Read-only investigators are cheap and parallel; writers are serialised by file. A
reviewer must not be the author. Route over-engineering findings through Ponytail
review.

**You own integration**: merge, resolve conflicts, run the battery yourself, keep
`docs/04-STATUS.md` current. Never accept "done" on a subagent's word.

## 13. Definition of done

- [ ] A fresh clone builds; `cargo test --workspace` passes; the battery is green.
- [ ] Changed behaviour is covered by a test that fails for the reason it names.
- [ ] Anything a user sees was rendered and **looked at**, not only asserted on state.
- [ ] New public items are documented; behaviour-level changes have an ADR.
- [ ] Docs updated, commands run, links resolve.
- [ ] Ponytail's audit is clean, or every remaining finding is justified in writing.
- [ ] README credits and licence section complete; workspace clean.
- [ ] `docs/04-STATUS.md` states what changed and what is still broken, including
  the plaintext credential trade-off and any untested platform.

## 14. Conflict resolution

A direct user instruction wins, except safety-relevant rules — explain the
conflict rather than complying silently. Engineering judgement may override this
file when you can say why and leave a record.
