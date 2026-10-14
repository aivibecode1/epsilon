# Handoff — read this before your first change

You are picking up a two-day build that ran out of clock. **This file is the working guide for the
next agent**; it is deliberately separate from [`START-HERE.md`](START-HERE.md), which is written
for a human reviewer meeting the project for the first time.

Read in this order:

1. [`../AGENTS.md`](../AGENTS.md) — the locked contract. **Read it in full, every session.** It is
   short on purpose and it is not negotiable.
2. **This file** — what is true right now, what is verified, and what has bitten.
3. [`STATUS.md`](STATUS.md) — the living record: every milestone, every measurement, every open
   item. It is long; the **Known-broken** and **Next** sections are the ones you need.
4. [`plans/026-convergence.md`](plans/026-convergence.md) — the current plan: the 2026-10-09 audit's re-rank (R1–R8), with plan 025 dispositioned inside it.
5. [`plans/024-finishing-the-objective.md`](plans/024-finishing-the-objective.md) — the
   historical plan; several of its items are already landed and the rest are carried by 026.

---

## 1. What this is, in three sentences

Epsilon is a coding-agent harness in Rust: an agent core that reads, edits, runs commands and keeps
a resumable session. It has **one terminal IDE over one core** — a full-screen terminal
app (`epsilon-cli` / `epsilon-tui`: file tree, file viewer, agent conversation, palette)
plus headless, HTTP/SSE and stdio-RPC modes. (The native desktop IDE was removed in
ADR-039; its matrix is retired.) It began as a port of [pi](https://github.com/earendil-works/pi); [`../CREDITS.md`](../CREDITS.md)
names everything else it learned from.

**The architecture invariant that matters most** (`AGENTS.md` §2): the agent loop is `epsilon-core`'s
`Agent`, and `epsilon-runtime` is the *only* place a workspace plus a configuration becomes a
running agent (`build_runtime`, `resolve_launch`). A capability one mode has, every mode has,
because it is the same machinery behind a different draw call. **A second copy of composition is a
defect, not an optimisation.**

---

## 2. The state of the build, honestly

**The suite is green and the tree is clean.** Latest battery: see the *Latest green battery* line at
the top of `STATUS.md`; it names the log file under `audit/evidence/`. The convergence's closing
record is the row dated 2026-10-09 ("the convergence is done") and the plan that drove it is
[`plans/026`](plans/026-convergence.md).

**The battery is named in one place only — the line at the top of `STATUS.md`.** This file used to
restate its totals and drifted twice (the 2026-10-05 snapshot's stale count, and a 13 carried where the
rework's log measured 15), so the numbers live where the log is named and nowhere else. The count is never a fact; the command that
produced it is.

**Do not trust a number in a document over the command that produced it.** Every count in
`STATUS.md` is attributed to a log; if a number matters, open the log, or re-run the command. This
repository has lost real time to summaries that drifted from the tree, three times, and each case is
written down.

### What is genuinely done

- **One core, one terminal IDE, one composition point, and one command registry.** The command half
  lives in `epsilon-contracts` and every mode reads it; the *keybindings* half closed with the
  shared grammar (M315). The *flat* `REGISTRY` of `Action` rows is not built — see the plan.
- **The removed desktop was a real native window** and it had been **seen** (retrieved):
  `audit/evidence/desktop-frames-2026-10-04/` holds a rendered 1440×900 frame. The desktop
  itself is gone (ADR-039); the frames stay as evidence of what was verified while it lived.
- **Approvals, permissions (including presets), checkpoints/rollback, sessions/resume, goals,
  subagents, MCP, skills, model and provider switching, cost tracking** all exist and are exercised by
  the suite. The terminal's failure rows have their own severity; the tree (F6) and file viewer
  (Enter) are covered with frames read at 80×24 and 100×30.

### What is NOT done — the short version

The plan has the detail and the evidence. In rough priority order:

1. **A real screen reader has never been run, and no dated human keyboard-only pass exists.** UI
   Automation is one layer *below* a reader: it proves the platform exposes names and roles, not that
   anything announces them. Both items need a person, and they have slipped every round (plan 026 R5).
2. **The desktop is gone, and its open rows went with it.** The DSH matrix is retired
   (ADR-039, frozen history); the \~70 Electron-chrome rows and the portable remainder are
   moot — a window that does not exist has no gaps. The live question is the pi matrix.
3. **Plaintext credentials still sit in `auth.json`** against locked `AGENTS.md` §6; proposal 7 is
   unanswered and the fix needs the owner's signature (plan 026 R6).
4. **The release is untagged.** Preview install was verified (round 73); no artifact is published
   (plan 026 R7).
5. **The compaction digest's replay and markdown halves.** A resumed session hydrates from *messages*
   while the digest lives in the store's compaction entry, so `/resume` shows `[context compacted]`
   without opening it (P-054/P-183 stay `partial`).
6. **The pi matrix's missing rows.** The DSH Desktop matrix's 275 open rows retired with the
   desktop (item 2); what remains is pi-tui's `missing` set, which is roadmap, not a defect list.

---

## 3. How to work here

### The battery

```sh
pwsh -File scripts/gate.ps1 <label>          # Windows — use this one here
```

Fifteen legs, and the log names them in order: ten repository checks (links, `AGENTS.md`
lock, lint tables, parity citations, citation placement, reference citations, verb counts,
parity counts, line endings, control bytes),
then `fmt`, `clippy -D warnings`, the full test suite, the offline demo and the real-feature
check. (This line said "Ten legs" — five repository checks — from before the citation legs
landed, then "Thirteen legs, eight repository checks" when the citation legs landed; the
seventh audit found the second also stale, and the count is now taken from the log's own
`=====` lines rather than remembered.)

- **Give it a timeout above 600 s.** The test leg alone runs several minutes.
- **Never re-use a label.** `gate.ps1` truncates the log, so a reused label destroys the failed
  attempt's evidence.
- **Never edit source while a battery runs.** The log then describes a tree that no longer exists.
- Copy evidence you want to keep out of `target/`, which is disposable.

### This machine — the rules that cost real time to learn

- **One build owner at a time.** A gate round, a delegated build and a `cargo install` must not
  share the machine. Use `CARGO_BUILD_JOBS=1`.
- **Build outside the repository.** `CARGO_TARGET_DIR` goes to `%TEMP%\epsilon-target` (set by
  `gate.ps1`). Writing build output inside this repository makes `cl.exe` fail with `D8050`, which
  cargo reports as a broken *dependency*.
- **Disk is tight.** Debug info off (`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`)
  runs a battery in ~4 GB instead of ~33 GB. **Name the knob wherever you cite a battery.**
- `bash` here is WSL and has no Windows `cargo`. Use `gate.ps1`.
- **`LNK1104` on a freshly linked test executable is a machine fault, not a defect** — a
  security-software block bound to one exact filename. Never change security configuration from an
  agent session. The remedy is fresh test-profile metadata
  (`CARGO_PROFILE_TEST_DEBUG=<a value this machine has not used>`).

### The disciplines that have earned their place

Each of these caught something real; the evidence is in `STATUS.md`.

- **RED first.** Write the test, watch it fail *for the reason it names*, then fix. A test that
  never failed has not been shown to test anything.
- **Run the negative control.** Restore the defect and confirm the test fails. Then restore the fix.
  Half the milestones here carry the control's output verbatim in a commit message.
- **Assert on the frame, not on state, for anything a user sees.** Two features once shipped that
  were asserted on state and never rendered. `Audit/evidence/` holds rendered terminal frames
  (the desktop frames beside them are retired history); `EPSILON_FRAME_DIR` puts new ones beside them.
- **Grep for callers, then check at least one is production code.** A definition plus a test is not
  a feature.
- **A check that cannot see a thing is indistinguishable from a check that found nothing.** This
  repository has been fooled by it at least four times — a probe that matched the wrong window and
  exited 0, a checker resolving against a directory that did not exist, a guard test pinning the
  very limitation it was written to catch. When you add a check, ask what the broken version would
  print, and confirm it differs.
- **A finding is a claim about a tree.** `AGENTS.md` is locked; defects, docs and line numbers drift.
  Re-verify before acting, and record what you found rather than only what you did.

---

## 4. Things that will mislead you if nobody tells you

- **The desktop window (removed in ADR-039) rendered blank when launched normally.** No wgpu backend *presented* here:
  Vulkan finds the adapter and then takes the process down, DX12 stack-overruns, GL works only
  through a **swapchain-free offscreen path**. That is why `crates/epsilon-desktop/src/frame_raster.rs`
  existed and why `audit/evidence/desktop-frames-2026-10-04/` has an image at all. Retired history —
  the terminal frames are the live evidence; do not build new desktop guidance on this.
- **The TUI frame dumps are character-only** (`crates/epsilon-tui/src/app/frame_dump.rs`,
  `#[ignore]`d tests, `EPSILON_FRAME_DIR`). They cannot show colour or contrast. The ConPTY suite in
  `crates/epsilon-cli/tests/terminal.rs` drives the **real binary in a real pseudo-console** and is
  the stronger evidence.
- **`accesskit_node_builder` was get-or-create (retired with the desktop, ADR-039).** Calling it for a node egui did not build *created*
  one. This is why one unnamed `ScrollBar` stayed unnamed, and why the fix was not a one-liner.
  Kept as a caution for any future native surface, not as live guidance.
- **The desktop's accessibility dump in `audit/evidence/desktop-accesskit-tree-2026-10-04.md`
  predates M301–M303 and has no producer.** It records a `Role::Unknown` and a `Role::Pane` that no
  longer exist. Treat it as history; `audit/evidence/desktop-uia-tree-2026-10-04.txt` is the
  reproducible one.
- **`docs/README.md`'s milestone pointer list stops at M276, and M277 onward have no dev log or
  history row.** That is a recorded decision, not an omission. Work after M276 lives in `STATUS.md`
  and `CHANGELOG.md`.

---

## 5. Working with subagents

`AGENTS.md` §12 is the rule. Three things this session learned the hard way:

- **Every brief begins "read and obey `AGENTS.md`".** Subagents never edit it.
- **A subagent's finding is a claim, and some are wrong.** In this session one reported a
  nine-item list where the file correctly said ten (checked against `gate.ps1`); another's premise
  was right but its prescribed fix would have created phantom accessibility nodes. **Re-run the
  check yourself before acting.**
- **Agents can also be right in a way that corrects you.** Two read-only passes corrected claims
  written in `STATUS.md`, and one found a defect nobody had written down (the unbounded chrome
  lines). Route review through someone who did not write the code.

---

## 6. Where to look for what

| You need | Go to |
|---|---|
| The rules | [`../AGENTS.md`](../AGENTS.md) |
| What is true now, and what is broken | [`STATUS.md`](STATUS.md) |
| What is left to do, prioritised | [`plans/026-convergence.md`](plans/026-convergence.md) (024 and 025 are history, dispositioned inside 026) |
| A human's tour of the project | [`START-HERE.md`](START-HERE.md) |
| Architecture and dependency graph | [`design/epsilon-architecture.md`](design/epsilon-architecture.md) |
| Why a decision was made | [`design/epsilon-decisions.md`](design/epsilon-decisions.md) |
| Battery logs and measurements | [`../audit/evidence/`](../audit/evidence/) |
| Rendered proof (terminal frames; retired desktop frames) | [`../audit/evidence/tui-frames-2026-10-04/`](../audit/evidence/tui-frames-2026-10-04/), [`../audit/evidence/desktop-frames-2026-10-04/`](../audit/evidence/desktop-frames-2026-10-04/) |
| Every gate and probe, and why each exists | [`../scripts/README.md`](../scripts/README.md) |
