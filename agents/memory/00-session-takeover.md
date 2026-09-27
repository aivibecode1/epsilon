# Session takeover — full restart (2026-09-28)

**Status: audit phase. Code touched so far: ONE new test file (a measurement, not a fix).**

## What I was told

Restart the study from scratch. Treat `./audit`, the skills, and the inherited notes as
unverified. Order: (1) honest evidence-based audit, no product code; (2) restructure repo +
docs; (3) implement — TUI Windows freeze first, then context/cache/cost accuracy; (4) finish
the pi-agent port; (5) take the best architectural ideas from `references/`. Engineering
judgment overrides `AGENTS.md`; edit it and report the delta.

## Verified repo state (my own commands)

- HEAD `d63e604`, `main`, ahead of origin/main by 301. Working tree had 8 modified files at
  session start — ALL inherited from the previous agent (`AGENTS.md`,
  `crates/epsilon-cli/tests/terminal.rs`, 6 files in `.agents/skills/ratatui-tui/`) plus
  untracked `docs/design/epsilon-architecture.draft.md`. Not mine; decide before committing.
  The `terminal.rs` M179 change (fall back to `CARGO_BIN_EXE_epsilon`) is correct — keep it.
- Rust LOC 107,837 / **12 crates** (tui 28,119 · llm 21,615 · core 19,233 · tools 10,613 ·
  session 9,402 · cli 7,166 …). I first counted 11 by missing `epsilon-safety`; the repo is
  internally consistent at 12 (Cargo.toml, `crates/`, README, AGENTS.md all agree). **My
  earlier "AGENTS.md says 12, there are 11" note was wrong — do not repeat it.**
- `audit/` is INCOMPLETE: only `README.md`, `01`, `03`, `04` exist. The README's table
  promises `02-port-fidelity`, `05-metrics`, `06-references`, `07-plan` — **none exist**.
  `audit/evidence/` is 13 MB of mostly stale logs; `audit/history/` has 14 old assessments.
- 171 milestone docs, 65 KB CHANGELOG, `docs/` with design/history/milestones/qa/plans/
  research/user/handoff. References: pi-0.85.1, hermes-agent-2026.9.14, opencode-1.18.31,
  deepseek-harness-dsh.
- Disk C: 37 G free (93%). All builds go to `CARGO_TARGET_DIR=D:/eps-target`, `JOBS=1`, wiped
  after each round.

## THE FREEZE — what I actually measured (do not inherit the old story)

Ran `scripts/console-probe.py` against a real conhost, twice: a fresh **debug** build
(`D:/eps-target/debug/epsilon.exe`) and the **installed release binary**
(`~/.cargo/bin/epsilon.exe`, which is what the user actually runs — 20 MB vs 31 MB).

**Typing works. Submitting works. The turn completes.** Concretely:
- `typed_screen` showed `• say hi`; the cursor advanced exactly 6 columns.
- `--early` (typing *during* startup) also worked: `• hello second`.
- `turn_screen` showed the run streaming with real header numbers, then the permission
  dialog; pressing `y` produced `Run complete.`
- The previous audit's claim that "the user is never told to press `y`" is **wrong**:
  `app/panels.rs:637-647` paints `y allow once / a always / n deny / e deny+guidance`, and
  the footer repeats it (`Hints::approval()`).

**`--tui-bench`: per-frame draw is ~2.9 ms @20 msgs, ~3.1 ms @100, ~3.7 ms @300.** The draw
path is NOT the freeze. But the bench calls `draw_frame` directly and its `bench_app` holds
no `Agent`, so it **never runs the per-frame context mirror** — it understates a real frame.

**Confirmed defect (measured; new test `crates/epsilon-core/tests/context_mirror_cost.rs`):**
`app/event.rs:169` calls `App::mirror_context_size` every drawn frame →
`Agent::estimated_context_tokens` (`core/lib.rs:1934`) → `self.history()` — a **full deep
clone of the whole transcript** (`lib.rs:1114-1116`).
Measured: **16.9 µs @20 msgs → 216.7 µs @200 msgs = 12.9× cost for 10× transcript.** So it
IS a whole-transcript scan, as suspected — but at 216 µs it is ~0.7% of a 33 ms frame and
does **not by itself explain a total freeze**. Honest conclusion: real defect, wrong prime
suspect for the freeze. The second test in that file fails on purpose until it is fixed —
it is the tripwire for the fix.

**The real gap:** the faux provider emits every event instantly
(`llm/src/faux.rs:145-148`, `pending.pop_front()`, no delay), so **no test or probe has ever
run the TUI while a turn is genuinely in flight over seconds.** `SlowProvider`
(`core/src/testing.rs:71`) only delays *before* the stream opens — it cannot simulate a long
streaming turn — and it is reachable from rpc_mode and server but **not from the TUI**.
That missing harness is why this class of bug survives. **Building it is the first
implementation task.**

## Still open

- Subagent findings (TUI root cause, port fidelity, metrics, layout) — collecting.
- Untested: behaviour during a multi-second streaming turn, and at 60-column width.

## Machine facts

- `pwsh -File scripts/gate.ps1 <label>` is the battery that works here.
- Build owner: one at a time. Avira has pinned test-artifact paths before — never change AV
  config. Never use bare `/tmp` for native tools; use `$TMPDIR`.

## Next safe action

Write `audit/` properly: 02 port fidelity, 05 metrics, 06 references, 07 plan; correct 01/03/04
where measurement contradicts them. Then restructure, then implement — harness first.
