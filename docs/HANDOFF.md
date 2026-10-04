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
4. [`plans/024-finishing-the-objective.md`](plans/024-finishing-the-objective.md) — the prioritised
   plan for what is left, with the evidence for each item.

---

## 1. What this is, in three sentences

Epsilon is a coding-agent harness in Rust: an agent core that reads, edits, runs commands and keeps
a resumable session. It has **two front ends on one core** — a terminal UI (`epsilon-cli` /
`epsilon-tui`) and a **native desktop IDE** (`epsilon-desktop`, egui/eframe over winit + wgpu, no
webview). It began as a port of [pi](https://github.com/earendil-works/pi); [`../CREDITS.md`](../CREDITS.md)
names everything else it learned from.

**The architecture invariant that matters most** (`AGENTS.md` §2): the agent loop is `epsilon-core`'s
`Agent`, and `epsilon-runtime` is the *only* place a workspace plus a configuration becomes a
running agent (`build_runtime`, `resolve_launch`). A capability the desktop has, the terminal has,
because it is the same machinery behind a different draw call. **A second copy of composition is a
defect, not an optimisation.**

---

## 2. The state of the build, honestly

**The suite is green and the tree is clean.** Latest battery: see the *Latest green battery* line at
the top of `STATUS.md`; it names the log file under `audit/evidence/`. As of this handoff that was
`gate-m311final`, ten legs all `[exit=0]`, 2,044 passed / 0 failed / 8 ignored across 78 binaries.

**Do not trust a number in a document over the command that produced it.** Every count in
`STATUS.md` is attributed to a log; if a number matters, open the log, or re-run the command. This
repository has lost real time to summaries that drifted from the tree, three times, and each case is
written down.

### What is genuinely done

- **One core, two front ends, and one command registry half-moved.** The command half lives in
  `epsilon-contracts` and both front ends read it. The *flat* `REGISTRY` of `Action` rows is not
  built — see the plan.
- **The desktop is a real native window** and it has been **seen**: `audit/evidence/desktop-frames-2026-10-04/`
  holds a rendered 1440×900 frame. Getting there is worth understanding before you touch rendering —
  see §4.
- **Its accessibility tree is known to reach the operating system**, read through Windows UI
  Automation from a separate process (`scripts/desktop-a11y-probe.ps1`).
- **Approvals, permissions, checkpoints/rollback, sessions/resume, goals, subagents, MCP, skills,
  model and provider switching, cost tracking** all exist and are exercised by the suite.

### What is NOT done — the short version

The plan has the detail and the evidence. In rough priority order:

1. **A real screen reader has never been run.** UI Automation is one layer *below* a reader: it
   proves the platform exposes names and roles, not that anything announces them.
2. **Lease-takeover detection** — two writers can still share one session file. **Read the plan
   before attempting this**; the obvious fix makes the product worse and the reason is written down.
3. **The denied-tools filter**, **`/model` auto-fetch targeting**, **the desktop answering 3 of 38
   commands**, **the flat `Action` registry**, **one unnamed `ScrollBar`**, and **Class B of the
   dead-code sweep** (50 items, each needing a judgement rather than a sweep).

---

## 3. How to work here

### The battery

```sh
pwsh -File scripts/gate.ps1 <label>          # Windows — use this one here
```

Ten legs: five repository checks (links, `AGENTS.md` lock, lint tables, line endings, control
bytes), then `fmt`, `clippy -D warnings`, the full test suite, the offline demo and the real-feature
check.

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
  were asserted on state and never rendered. `Audit/evidence/` holds rendered frames for both front
  ends; `EPSILON_FRAME_DIR` puts new ones beside them.
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

- **The desktop window renders blank when launched normally.** No wgpu backend *presents* here:
  Vulkan finds the adapter and then takes the process down, DX12 stack-overruns, GL works only
  through a **swapchain-free offscreen path**. That is why `crates/epsilon-desktop/src/frame_raster.rs`
  exists and why `audit/evidence/desktop-frames-2026-10-04/` has an image at all. **If you need to
  see the window, render it offscreen; do not try to fix presentation.**
- **The TUI frame dumps are character-only** (`crates/epsilon-tui/src/app/frame_dump.rs`,
  `#[ignore]`d tests, `EPSILON_FRAME_DIR`). They cannot show colour or contrast. The ConPTY suite in
  `crates/epsilon-cli/tests/terminal.rs` drives the **real binary in a real pseudo-console** and is
  the stronger evidence.
- **`accesskit_node_builder` is get-or-create.** Calling it for a node egui did not build *creates*
  one. This is why one unnamed `ScrollBar` is still unnamed, and why the fix is not a one-liner.
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
| What is left to do, prioritised | [`plans/024-finishing-the-objective.md`](plans/024-finishing-the-objective.md) |
| A human's tour of the project | [`START-HERE.md`](START-HERE.md) · [Arabic](START-HERE.ar.md) |
| Architecture and dependency graph | [`design/epsilon-architecture.md`](design/epsilon-architecture.md) |
| Why a decision was made | [`design/epsilon-decisions.md`](design/epsilon-decisions.md) |
| Battery logs and measurements | [`../audit/evidence/`](../audit/evidence/) |
| Rendered proof for both front ends | [`../audit/evidence/desktop-frames-2026-10-04/`](../audit/evidence/desktop-frames-2026-10-04/), [`../audit/evidence/tui-frames-2026-10-04/`](../audit/evidence/tui-frames-2026-10-04/) |
| Every gate and probe, and why each exists | [`../scripts/README.md`](../scripts/README.md) |
