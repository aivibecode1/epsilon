# AGENTS.md — Working in the Epsilon repository

**Version 4.0 · 2026-10-12**

> **LOCKED: maintainer-owned; agents never edit this file.**
> Its SHA-256 is in `AGENTS.md.sha256` and `scripts/check-agents-lock.py` fails CI when they
> differ. If you believe this file is wrong, append a proposal to
> [`docs/AGENTS-PROPOSALS.md`](docs/AGENTS-PROPOSALS.md) and say so in your final report.
>
> v4 was rewritten from the tree under explicit owner authorisation to unlock, rewrite and
> relock, then relocked. v3 → v4 retires the desktop: the crate is deleted (`8992fec`,
> ADR-039), so the two-front-ends language, the native-desktop rule, the `SUPPORTED`
> subset, the 14-crate map, the 1.95 split and the desktop commands are gone. Evidence:
> the v4 application draft (`docs/AGENTS-v4-DRAFT.md`).

This file is the permanent reference for anyone — human or agent — changing this repository.
It holds only what stays true. Anything that moves (progress, open work, decisions in flight)
belongs in [`docs/STATUS.md`](docs/STATUS.md). For a guided tour of the project read
[`docs/START-HERE.md`](docs/START-HERE.md).

---

## 1. What this project is

Epsilon is an agent harness with **one core and one terminal IDE**:

- **`epsilon-cli` / `epsilon-tui`** — a full-screen terminal IDE (file tree, file viewer
  and editor, agent conversation, command palette), plus headless, HTTP/SSE and
  stdio-RPC modes.

It began as a port of [pi](https://github.com/earendil-works/pi) and has since taken ideas
from DeepSeek Harness, Hermes Agent, opencode and codewhale-ratatui. See
[`CREDITS.md`](CREDITS.md) and [`docs/references/`](docs/references/README.md).

(The native desktop IDE was removed in ADR-039; its parity matrix is frozen history under
`docs/parity/dsh-desktop.md`. A return comes back as a new ADR, never as a quiet re-add.)

## 2. Architecture invariants

These do not change without an ADR in
[`docs/design/epsilon-decisions.md`](docs/design/epsilon-decisions.md).

1. **One core.** The agent loop is `epsilon-core`'s `Agent`. The terminal IDE and the
   headless/HTTP/RPC modes are *modes on it*, not reimplementations. A capability one mode
   has, every mode has, because it is the same machinery behind a different draw call.
2. **One composition point.** `epsilon-runtime` is the only place a workspace plus a
   configuration becomes a running agent (`build_runtime`, `resolve_launch`). A second copy of
   composition is a defect, not an optimisation (ADR-021).
3. **No desktop frontend.** ADR-039 removed `epsilon-desktop` (egui/eframe) with its crate,
   its pins and its matrix. The `WINDOW` keybinding table is retained deliberately for the
   TUI IDE, not as a second frontend. A new window comes back only through a new ADR with
   its own battery shape — never as a quiet re-add, which the `check-verb-counts` removal
   guard fails on purpose.
4. **One command registry.** Slash commands (`COMMAND_HEADS`) and palette entries (derived
   from `COMMAND_HEADS`) and keybindings (the `TERMINAL` table, plus the retained `WINDOW`
   table) are declared once in `epsilon-contracts` and read by every mode, with per-mode
   capability declared as data rather than as a second hand-maintained list.
5. **No library depends on `epsilon-cli`.** Cargo rejects cycles; it does not stop you adding an
   edge nobody meant. `epsilon-cli` may depend on the others; the reverse is a review item.
6. **A refusal path never exits the process.** Functions shared across modes return an error;
   they do not `die()`. This is why `resolve_launch` returns `Err(String)` where the CLI once
   called `die()`.
7. **Nothing a UI advertises may be a no-op.** A bound key the UI does not act on says so; a
   palette entry names a command that exists and works. A silent no-op is worse than a missing
   feature, because a missing one does not lie.
8. **Types that cross a wire are part of the contract.** A wire name is verified by sending a
   real frame, never by asserting serde's output against the variant list.

## 3. Repository map

```
crates/
  epsilon-contracts   shared types and traits; zero internal dependencies
  epsilon-llm         provider protocols, faux provider, routing, catalog, auth, retry
  epsilon-session     JSONL storage, branching, leases, repair
  epsilon-core        the agent loop, scheduling, context assembly, compaction, tasks
  epsilon-tools       native tools, shell adapter, checkpoints, goals
  epsilon-safety      permission engine, approvals, sandbox backends
  epsilon-lsp         optional language-server client and diagnostics
  epsilon-ext         process-extension protocol and host
  epsilon-mcp         MCP client and tool adapter
  epsilon-runtime     COMPOSITION: the one place a config becomes a running agent
  epsilon-tui         terminal IDE: tree, viewer/editor, transcript, palette (ratatui + crossterm)
  epsilon-server      HTTP/SSE frontend
  epsilon-cli         the binary: argument parsing and mode dispatch
docs/                 START-HERE.md, STATUS.md, design/, references/, parity/, plans/, milestones/, user/
scripts/              gate.ps1 / gate.sh, check-*.py, console-probe.py
references/           read-only reference harnesses. Never modify, move or delete.
```

`references/` is **input**. It is not part of the build and not committed.

## 4. Build, test, lint, run

Toolchain: **Rust 1.88+**, uniformly — every crate inherits the workspace `rust-version`
and no manifest overrides it.

```sh
cargo check --workspace --all-targets
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings     # zero warnings, no exceptions
cargo test --workspace
cargo run -q -p epsilon-cli -- --demo                      # offline, scripted, no provider needed
cargo check -p epsilon-cli --features real
cargo run -q -p epsilon-cli                               # the terminal UI
```

**The battery runs all of it plus ten repository gates, fifteen legs in all:**

```sh
pwsh -File scripts/gate.ps1 <label>    # Windows — use this one here
bash scripts/gate.sh <label>           # POSIX and CI
pwsh -File scripts/gate-until-pass.ps1 -Label <label>   # retry with a fingerprint, stops on a repeat
```

Legs, in order: `check-links`, `check-agents-lock`, `check-lint-tables`,
`check-parity-citations`, `check-citation-placement`, `check-reference-citations`,
`check-verb-counts`, `check-parity-counts`, `check-line-endings`, `check-control-bytes`,
`fmt`, `clippy`, `test`, `demo`, `real-check`. The log lands at
`target/gate-<label>.log`. **Give it a timeout above 600 s** — the test leg alone runs several
minutes. Copy evidence you want to keep out of `target/`, which is disposable.

**Never re-use a battery label.** `gate.ps1` truncates the log, so a reused label destroys the
failed attempt's evidence. **Never edit source while a battery runs** — the log then describes
a tree that no longer exists.

### This machine

- **One build owner at a time.** A gate round, a delegated build and a `cargo install` must not
  share the machine. Use `CARGO_BUILD_JOBS=1`; parallel jobs have failed here with
  `0xc0000142` compiler-startup contention.
- **Build outside the repository.** `CARGO_TARGET_DIR` goes to `%TEMP%\epsilon-target`, set by
  `gate.ps1`. Writing build output inside this repository makes `cl.exe` fail with
  `D8050: cannot execute c1.dll`, which cargo reports as a broken *dependency*.
  Any shell build must export both variables first; a `ring`/`cl.exe` failure after
  forgetting them is a machine fault, not a defect — set the variables and re-run.
- **`LNK1104` on a freshly linked test executable is a machine fault, not a defect.** Avira
  flags some paths and the link fails on a file that does not exist. Never change security
  configuration from an agent session. Rebuild the test profile under fresh metadata
  (`CARGO_PROFILE_TEST_DEBUG=1`) and disclose the knob in the log.
- **Disk is tight.** Turn debug info off to run a full battery in ~4 GB instead of ~33 GB
  (`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`), and turn it back on for the round
  that debugs a failure. Name the knob wherever you cite the battery.
- `bash` here is WSL and may not have a working Windows `cargo`. Prefer `gate.ps1`; if your
  shell does have one, the build-env rules above still apply.
- The repository is **LF** in the index and on disk (`.gitattributes`: `* text=auto eol=lf`).

## 5. Engineering rules

- **Ask Ponytail first.** Before adding code, a dependency or an abstraction, walk the ladder:
  does it need to exist; is it already in this codebase; does the standard library do it; does
  the platform do it; is a dependency already installed enough. Then write the minimum. Mark a
  deliberate shortcut with a `ponytail:` comment naming the ceiling and the upgrade trigger.
  Never let it block a justified architectural improvement, validation at a trust boundary,
  accessibility, or protection against data loss.
- **Reuse before you add.** A helper, type or pattern already three files over is the most
  common slop. If two modes need the same decision, it belongs in the core, not in both.
- **Smallest correct change.** Deletion beats addition. A refactor that also changes behaviour
  cannot be reviewed as a refactor.
- **Rust:** `unsafe_code = "forbid"` workspace-wide; `missing_docs` is on — write docs you would
  want to inherit. Direct dependencies pinned exact (`=x.y.z`) in `[workspace.dependencies]`, and
  no new dependency without a stated reason. New crates need an ADR first.
- **Comments explain why**, not what. Cite the reference implementation when semantics come
  from one, with a real `file:line`, not a memory of it.
- **Behaviour-level changes need an ADR entry or amendment.**

### Evidence discipline

These failure modes have each cost this repository real time. They share one shape: *something
reports success while the thing you wanted did not happen.*

- **A test that passes is not a claim that is true.** For anything a user sees, drive the real
  draw path and assert on the **frame**. Asserting on state is how two features shipped that
  were never rendered.
- **Grep for callers, then check that at least one is production code.** A definition plus a
  test is not a feature. This repository has shipped primitives that were correct, tested, and
  unreachable from any surface.
- **A control that passes is a control you have not proven.** Ask what the broken version would
  print and confirm it differs.
- **Measure the number the code reads**, not the one you reasoned about.
- **Check whether the frame is the bug** before concluding the code is — against the reference,
  never against your own naming.
- **Never quote a test count you did not measure.**
- **A frame is not asserted until you have looked at it.** Render one and read the screen at
  80×24 and 100×30.

## 6. Security and data loss

- Every side-effecting tool call passes the permission gate. Deny is monotonic: memory can never
  resurrect a denial. An unknown capability is `Ask`, never `Allow`.
- The approval channel **fails closed** — a dropped responder resolves to deny.
- Untrusted text (model output, tool output, file contents, session names) is stripped of
  control and bidi characters before it reaches a screen.
- **Never write a credential to disk in plaintext.** Secrets belong in the OS credential store.
  Never accept one on a command line.
  **Known open violation:** provider credentials are currently stored in
  `<config dir>/auth.json` (`crates/epsilon-llm/src/auth.rs`), against this rule. The fix
  needs the owner's signature (plan 026 R6, proposal 7); until it lands, no new plaintext
  secret may be added and the rule stays as stated.
- **Never lose a user's work.** Confirm before discarding or overwriting; keep a recoverable
  copy; a refusal path must leave the file untouched.
- Validate input at every boundary — paths, RPC frames, HTTP bodies, MCP payloads. A path
  crossing into the filesystem is normalised and checked for escape, not joined and trusted.
- Minimal dependencies. A new dependency is a supply-chain decision, not a convenience.

## 7. Testing

- **Faux provider first.** Full-loop scenarios run offline against scripted steps. Real-provider
  tests are `#[ignore]`d behind an env var; nothing hits a paid API by default.
- Deterministic time. Use the project's slow provider; never sleep and hope.
- Each crate owns integration tests for its **failure** paths, not only its happy path.
- A test is not done until it fails for the reason it names. Prove the causation when you fix
  something a test caught.

## 8. UI and accessibility standard

Applies to the terminal IDE first and to every other mode as far as each can.

- **WCAG 2.2 AA is the floor**, not a stretch goal.
- **Full keyboard operation**, logical focus order, visible focus, and **no keyboard traps**.
  Every control reachable by mouse is reachable and operable by keyboard.
- **Screen-reader names and roles** through the platform accessibility API. Painter-drawn
  surfaces emit no accessibility nodes — anything a screen reader must reach has to be a real
  widget or carry registered metadata.
- **Contrast**: body text ≥ 4.5:1, secondary text and UI boundaries ≥ 3:1, in **every** theme.
  Never rely on colour alone to carry meaning.
- Honour **reduced motion**, **OS dark/light**, **high contrast**, **text scaling** and **DPI**.
- **Design system**: colour, type and spacing tokens in one place; one type scale; consistent
  components.
- **Every async view has states**: loading, empty, error, cancelled, long-running. No silent
  failures. No data loss — recovery, and confirmation before discarding.
- **Performance**: fast startup, virtualised long transcripts and large files, no blocking of
  the UI thread. A frame cost that is measured is worth more than one that is assumed.

## 9. Documentation

- `docs/START-HERE.md` is the reading guide for a newcomer. Keep it true.
- **Document intent, structure and decisions — not what the code already says.** Code comments
  explain why.
- Plain and concrete. Short sentences. Define a term at first use. Examples over adjectives.
  No marketing, no filler, no generic phrasing.
- **Every command in the docs must have been run. Every link must resolve.**
  `scripts/check-links.py` is a gate; the commands are on you.
- New documentation files are added to [`docs/README.md`](docs/README.md), the map.

## 10. Credits and licences

- **Attribution is mandatory and respectful.** Take authors, links and licences from each
  project's own README, LICENSE and package metadata — never from memory. If uncertain, say
  so.
- Never belittle a reference. Describe differences as differences in goals and design
  philosophy. Where we build on Pi, say we build on Pi's extensibility.
- **Ideas versus code.** Before copying or adapting, check licence compatibility. Compatible →
  keep the original copyright and licence notices in `THIRD_PARTY_NOTICES` and mark adapted
  files with a header naming the source. Unclear → reimplement from the idea. Never present
  others' work as ours, and never take a name, logo, mascot or brand.
- `./references` is read-only input: never modify, move or delete it.

## 11. Workspace hygiene

- Remove caches, temporary files and generated output as soon as they are useless. Keep source
  and the build the project needs — never several experimental builds.
- Before deleting anything, check `git status` that it is untracked and regenerable. **Never run
  a blanket clean** (`git clean -fdx`, broad `rm -rf`).
- Protect `references/`, `.git`, `AGENTS.md`, `AGENTS.md.sha256`, lockfiles and user data.
- Do not push, publish, or touch any remote. Never commit secrets.

## 12. Subagents

Every brief begins: **read and obey `AGENTS.md`**. Subagents never edit `AGENTS.md`.

Give each one a reason to exist, a narrow brief, explicit file ownership (**never two writers on
the same files**), a defined output, and the checks it must run. Scale parallelism to the
machine — one build owner at a time.

Read-only investigators are cheap and parallel; writers are serialised by file. A reviewer must
not be the author. Route over-engineering findings through Ponytail review.

**You own integration**: merge, resolve conflicts, run the full battery yourself, keep
`docs/STATUS.md` current. Never accept "done" on a subagent's word.

## 13. Definition of done

- [ ] A fresh clone builds; `cargo test --workspace` passes; the battery is green.
- [ ] Changed behaviour is covered by a test that fails for the reason it names.
- [ ] Anything a user sees was rendered and **looked at**, not only asserted on state.
- [ ] New public items are documented; behaviour-level changes have an ADR.
- [ ] Docs updated and their commands actually run; links resolve.
- [ ] Ponytail's audit is clean, or each remaining finding is justified in writing.
- [ ] `CREDITS.md` complete; `AGENTS.md` lock check passing; workspace clean.
- [ ] `docs/STATUS.md` says what changed and what is still broken.

## 14. Conflict resolution

Direct user instruction wins, except safety-relevant rules — explain the conflict rather than
complying silently. Your engineering judgment overrides this file when you have a good reason
and can say what it is.
