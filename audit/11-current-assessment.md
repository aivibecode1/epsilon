# 11 — Current assessment and the plan that follows

**Written:** 2026-09-19, working tree over `0548e9f`.
**Replaces as the live assessment:** [09 — source revalidation](09-revalidation.md),
which covered the tree at `80b32eb`. 09's findings are not withdrawn; they are
folded in below with their current status.

This file is the honest answer to three questions: what is actually true about
this codebase right now, what the evidence for that is, and what to do next in
what order. Where an earlier document claims something this file contradicts,
this file wins and says why.

## How this assessment was produced

Four independent passes, each read-only. The reports are checked in beside this
file rather than left in a scratch directory, because a finding that only exists
in a temporary folder is a finding the next reader has to redo:

| Pass | Method | Artifact |
|---|---|---|
| Test-suite credibility | Read all 884 test bodies; hand-verified every candidate finding | [`evidence/test-suite-audit.md`](evidence/test-suite-audit.md) |
| pi port fidelity | Read pi 0.85.1's loop, prompt, skills, compaction, tools, streaming; compared against the Rust counterparts | [`evidence/port-fidelity-audit.md`](evidence/port-fidelity-audit.md) |
| Reference design study | Read pi, opencode and hermes terminal UIs plus DSH's token layer | [`evidence/reference-tui-design-spec.md`](evidence/reference-tui-design-spec.md), [`evidence/reference-harness-ideas.md`](evidence/reference-harness-ideas.md) |
| Gap and contradiction review | Reconciled `audit/00`–`10`, `docs/history.md`, milestone logs and `git log` | summarized below |

Plus execution: a full-workspace test run on a fresh profile, a strict-Clippy
run, and a terminal-integration probe that drives the built binary in a real
Windows console.

Corrections to the brief that this assessment accepts as fact:

- **DSH has no terminal UI.** `references/deepseek-harness-dsh-v0.1.6-alpha.2` is
  a React-DOM web GUI; it contributes a design-token discipline, not a TUI to
  compare against.
- **opencode's TUI is SolidJS on `@opentui/core`**, not Go. Its Windows console
  shim over `kernel32.dll` is still the sharpest Windows workaround of the four
  and is cited as such.

## 1. Where the codebase actually is

**Verified green.** On a fresh test profile, every workspace crate passes:
`cargo test --workspace` reports 0 failures across all 12 crates. `cargo fmt
--all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, the
offline demo and `cargo check -p epsilon-cli --features real` all exit 0.

**The test suite is materially better than this genre's norm.** 884 test
functions, one `#[ignore]`d, no `#[should_panic]`, no tautological assertions,
and exactly one body out of 884 with no assertion at all. That is not a claim the
genre supports by default and it should be stated plainly.

**The port is not complete.** `audit/03-port-plan.md` still carries capability
rows as Partial/Missing, and nothing in this assessment supports a complete-port
claim. What is faithful is the *shape*: the loop, the event taxonomy, the
terminal-frame contract, partial-output preservation, and the session and
compaction entry kinds all match pi. What diverges is the compression and
recovery machinery and the instruction-loading seams.

### What changed since 09

- **M95 paged reads landed** (`27cd677`). Reads that exceed the byte cap now
  page instead of refusing, with the write path refusing a wholesale overwrite of
  a file the agent has only partly seen.
- **M96 closed three port gaps** (`94df984`): the overflow classifier missed
  Anthropic's `request_too_large` spelling so the compaction-and-retry branch
  never fired; the compaction system prompt asserted a template it never sent;
  and a dead-code warning was silenced rather than papered over.
- **The TUI presentation layer was rebuilt** (`7a94ccd`, `59fdf2e`, `f3f82e6`,
  `7ae8673`). `crates/epsilon-tui/src/ui/` now owns theme, layout, metrics,
  header, transcript, composer, footer, overlay and shell, with 213 tests.
  Transparency is a data model (`ui/metrics.rs`), not a draw call.

### Verified on this machine, 2026-09-21

The owner's original report was that the TUI "shrinks and freezes" in CMD and
PowerShell. The ConPTY test cannot answer that question — under ConPTY the console
is emulated inside the terminal application, so no assertion it makes can observe a
console *window* changing size. So a second harness was built:
[`scripts/console-probe.py`](../scripts/console-probe.py) opens a real conhost
window, attaches to it, measures the buffer and window geometry before and after the
TUI starts, then types into the console input buffer and reads the screen back.

Result, in four geometries including a maximized window over a 9001-row scrollback:
**the window rectangle never changes, the frame renders, and typed text reaches the
composer.** The active buffer does lose its scrollback height when the alternate
screen is entered, which is what an alternate screen is for. Full table and raw
output: [`evidence/console-probe-2026-09-21.md`](evidence/console-probe-2026-09-21.md).

That does not retroactively make the report wrong. It was two real defects, both
fixed and both measured when they were found: a full frame redrawn per keystroke and
per streamed delta (M94 — a 99-character burst echoing at ~30 characters/second),
and every Windows `bash` call loading the user's PowerShell profile, 6.2 s for a
one-word `echo` (M98), which from inside the TUI is indistinguishable from a
deadlock.

One defect *did* reproduce while probing, and it is new:

- **A killed instance locks the next launch out for up to 30 seconds.** The session
  lease only calls a holder dead once its lock file is older than `STALE_AFTER`
  (30 s, against a 10 s heartbeat), so relaunching after closing the window or
  End-Tasking the process prints `another epsilon is already writing this session
  (pid=…)` and **exits**. Reproduced: the 80×25 probe run refused to start because
  `pid=9944` — the previous run's TUI, killed seconds earlier — still held the lock.
  Not fixed here: checking whether a pid is alive needs a platform call and this
  workspace forbids `unsafe` outside a reviewed per-crate override, so the fix is a
  design decision (a narrowly scoped liveness probe, or a refusal that waits out the
  remaining window with a countdown instead of exiting).

## 2. Gaps that remain, ranked

### P0 — correctness or safety

| # | Gap | Evidence | Why it matters |
|---|---|---|---|
| **G19** | **Reading a pi session emptied it, then rewrote the file in place.** `heal_legacy_line` expected `message.content` to be a *string*; pi writes an array of parts, so `as_str()` failed and `unwrap_or("")` silently produced an empty message. The same branch ignored `message.role` (all 990 messages in pi's fixture became *user* turns) and read `timestamp` as a number when pi writes an ISO string (every entry was re-dated to now). Running it for real: `epsilon --resume before-compaction.jsonl --print hi` → "resumed 369 entries", exit 0, **line 2's 668-character prompt was 0 characters**, and the header lost `provider`/`modelId`/`thinkingLevel`/`branchedFrom`. | [`evidence/pi-import-empties-messages-2026-09-22.md`](evidence/pi-import-empties-messages-2026-09-22.md) | **Closed in M114 + M115.** M114 (`d54dfb8`) reads roles, the content array and both timestamp spellings, and preserves what it cannot map — the fixture's prompt comes back whole (668 characters, resumed 372 entries; battery `m114a`). M115 adds the two halves that made the conversion *lossy and irreversible*: `SessionHeader` keeps the fields this build does not model (the M113 rule, one layer up, with the same as-JSON-not-as-bytes caveat), and `persist_migration` copies the previous bytes to `<file>.pre-epsilon` once, never overwriting an existing copy, with both CLI call sites naming the path. **What is still not done, and is a behaviour change rather than a bug:** conversion happens *in place* — the importer plan's import-as-copy rule would make `--resume` produce a new file and leave the original untouched, which carries a UX question (which file am I in now?). `model_change`/`thinking_level_change` also heal into `Custom`: preserved, not applied. |
| ~~G1~~ | **Closed** in M97 (`2d50813`): the `skill` tool now exists and is registered from the discovered list, and the catalog no longer advertises skills the model may not load (`87c5628`). | new tests in `crates/epsilon-tools/src/skill.rs` | — |
| G2 | F09 resolved filesystem containment is open | `audit/02-defect-ledger.md:157`; `agents/memory/f09-path-policy.md` | The native filesystem tools run inside the harness process; Windows Job Objects bound resources, not content. |
| G3 | **Re-audited 2026-09-21, and the row was half wrong.** F15 ("restore conflates skipped with deleted") is **refuted by the code it cites**: `restore` branches on `entry.existed` and refuses, `append` returns a `Result` that becomes `Recorded::Skipped`, and content is verified before an overwrite — the ledger was last written 2026-09-18 and M100/M104 rewrote that file on 2026-09-20. F07 is **confirmed**: `epsilon-core`'s `persist` does `let _ = st.append(...)` and `pop_last_message` does `let _ = s.pop_last()`, while the provider request reads the store projection, so a failed durable write leaves memory and the model view silently divergent. | [`evidence/g3-g4-checkpoint-restore-design.md`](evidence/g3-g4-checkpoint-restore-design.md) | Two things survive: a *skipped* capture is never reported to anyone (`files.rs` discards `record()`'s outcome), and `restore` still has no production caller — so this is latent, not live. |
| G4 | Unsupported final entries are truncated (F08) | `audit/02` F08 | **Closed in M108.** A final line that is *whole JSON* but not a shape this build knows (an unknown `kind.type`, a second header, bare JSON) is now refused with a typed `LineDiagnosis::UnknownLineShape`/`UnknownEntryType` and the file is left byte-identical; only a genuine fragment — a line that does not parse at all, which is what a kill mid-write leaves — is still dropped as debris. Position no longer decides whether data survives; a pre-fix RED was run by reverting the three gates. The header-deletion half was fixed earlier in M105. |

### P1 — behaviour a user would notice

| # | Gap | Evidence |
|---|---|---|
| ~~G5~~ | **Closed** in M99 (`542fa92`): `.gitignore` is honoured, with the matcher in `epsilon-core::ignore` so the tools and the TUI's `@`-file completion share one answer. | [`evidence/port-fidelity-audit.md`](evidence/port-fidelity-audit.md) D-9 | — |
| ~~G6~~ | **Closed** in `87c5628`: both the catalog and the tool filter it. | D-4 | — |
| ~~G7~~ | **Closed** in M101 (`df338b7`) for `SYSTEM.md` + `APPEND_SYSTEM.md` (both load; the project pair shadows the global one). **`.epsilon/prompts/` stays detected and unloaded**: pi's `prompts/*.md` are user-invoked slash templates, not system-prompt text, and the template feature is unported — the gate is kept so they cannot load untrusted when it lands. | D-7 | prompts templates still open |
| ~~G8~~ | **Closed** in M101 (`df338b7`): `AGENTS.override.md` is in the candidate list, ancestors load root-to-leaf, and `doctor` now reads the loader's own list instead of a second copy. | D-8 | — |
| ~~G9~~ | **Closed** in M102 (`989bd0a`): a length-stopped summary is refused and the caller falls back to context-free pruning. | D-5 | — |
| ~~G10~~ | **Wrong, and closed by diagnosis**: the TUI was repainting (the spinner advanced); the *tool call* was stalling six seconds on a PowerShell profile. See G15. | M98 `3e1d01b` | — |
| ~~G16~~ | **Closed in M106.** The cache-hit gauge divided the session-total cache reads by the latest prompt size, so it inflated every turn and pinned at `cache 100%`; `UiState` now carries the latest call's cache read beside `latest_prompt_tokens` and the rate divides like for like. The test that should have caught it was named for the right property and asserted the wrong model — it moved the denominator while holding the numerator fixed. | `crates/epsilon-tui/src/{ui/metrics.rs,state.rs}` | — |
| ~~G18~~ | **Closed in M111.** The cost estimate priced two of the four disjoint token buckets: `usage.input` excludes cached tokens by the providers' own split, so cache reads and cache writes were charged at zero and a cache-heavy session's cost was low by roughly the cached share times the input price. `ModelInfo` now carries both cache prices (sourced by the generator — 359 of 557 rows publish a read price, 114 a write price), the arithmetic moved out of the draw path into `crates/epsilon-tui/src/ui/cost.rs` with four tests, and a session that moved tokens the registry cannot price reports a **floor** (`$0.42+`) rather than a total. | `crates/epsilon-tui/src/{app.rs,ui/cost.rs,ui/metrics.rs}`, `crates/epsilon-llm/src/catalog.rs`, `scripts/regen_catalog.py` | Remaining: the header's numbers still have no legend on screen — the `+`, the `?` and `cache —` are documented in code and in the milestone, not in the UI. |

| ~~G15~~ | Every Windows `bash` call loaded the user's PowerShell profile — 6.2 s for a one-word `echo`, measured against 1.1 s with `-NoProfile`. **Closed** in M98 (`3e1d01b`); the probe lives on as `crates/epsilon-cli/tests/terminal.rs`'s sibling knowledge, and the measurement is in `shell_argv`'s doc comment. This is what "the TUI freezes" actually was. |
| ~~G21~~ | **Closed in M118.** (a) `find` promised "1000 results or 64KB" with no byte cap behind it; the cap is now real — `util::TOOL_OUTPUT_MAX_BYTES` = 50 KiB, pi's own `DEFAULT_MAX_BYTES` (`truncate.ts:12`), applied through pi's `truncateHead` shape — and the description says 50KB, which is both enforced and pi's number. (b) `glob`'s walker budget is announced instead of silent. (c) `ls` gained pi's byte cap too. Each notice names only the bound that actually fired: when bytes cut a listing, the "Use limit=2N" advice is suppressed, because raising a limit that is not what stopped you is worse than saying nothing. | `crates/epsilon-tools/src/{find.rs,search.rs,util.rs}`, the two `descriptions/*.txt`; batteries `m117a`/`m118a` | **Still not done, and now flagged rather than implied:** `glob` has *no* byte cap and makes no claim about one — so its 5000 matches still reach the model as ~150 KB where `find` stops at 50 KB. The asymmetry is deliberate-by-omission and should be closed by adding the cap, not by deleting `find`'s. |
| G20 | **The tool surface diverges from pi, and two of the divergences are defects rather than choices.** pi's default active set is four tools (`read`, `bash`, `edit`, `write`; `agent-session.ts:2808-2810`) and it offers eight built-ins, the others opt-in; Epsilon registers twelve plus `skill` with no subsetting. **Judgment, made deliberately and recorded here so nobody "fixes" it by accident:** a larger default surface is a *product* difference, not a fidelity bug — this harness's contract is its own, and removing capability to match a count would make it worse, not more faithful. The two defects inside the row were fixed in M117 (`find` announced its cap; `ls` gained pi's `limit`, default 500). | [`evidence/port-gap-recheck-2026-09-21.md`](evidence/port-gap-recheck-2026-09-21.md) row 5 | Leave the active-set difference and say why when a milestone touches it. The prompt-token cost of twelve tool descriptions is real and unmeasured. |
| G20 | **The tool surface diverges from pi, and two of the divergences are defects rather than choices.** pi's default active set is four tools (`read`, `bash`, `edit`, `write`; `agent-session.ts:2808-2810`) and it offers eight built-ins, the others opt-in; Epsilon registers twelve plus `skill` with no subsetting. **Judgment, made deliberately and recorded here so nobody "fixes" it by accident:** a larger default surface is a *product* difference, not a fidelity bug — this harness's contract is its own, and removing capability to match a count would make it worse, not more faithful. The two defects inside the row were fixed in M117 (`find` announced its cap; `ls` gained pi's `limit`, default 500). | [`evidence/port-gap-recheck-2026-09-21.md`](evidence/port-gap-recheck-2026-09-21.md) row 5 | Leave the active-set difference and say why when a milestone touches it. The prompt-token cost of twelve tool descriptions is real and unmeasured. |

### P2 — structure and teaching value

| # | Gap |
|---|---|
| ~~G11~~ | **Closed**: `app.rs` is **3,594 lines** from 6,700. The keyboard model, the event loop and all three test modules now live beside it (`app/keys.rs`, `app/event.rs`, `app/{tests,golden,composer_viewport_tests,tests_support}.rs`). |
| ~~G12~~ | **Closed** in `278d2ec`: every surface takes a passed-in `Theme` and the shim is deleted. |
| ~~G13~~ | **Closed**, in two layers. In-process: `app/golden.rs` drives the production `draw()` at 44x10 through 160x48 and asserts on the assembled frame. Process-level: `crates/epsilon-cli/tests/terminal.rs` spawns the real binary in a pseudo-console and asserts on the screen — startup, typing, the visibility of the gauges, a submitted turn, and a resize to 44x10. Run it with `EPSILON_BIN=target/debug/epsilon cargo test -p epsilon-cli --test terminal`; it skips loudly without that variable. |
| ~~G14~~ | **Closed** in M103: `epsilon-ext` went from 5 tests to 30, driving the real fixture process rather than a mock. It found a real defect — the frame cap only bounded a line whose newline had not yet arrived, so one oversized line came back as valid. |
| ~~G17~~ | **Closed in M107.** `crates/epsilon-tui/src/footer.rs` — a complete superseded footer, `pub` in a `pub mod`, so `clippy -D warnings` was structurally blind to it — is deleted; its one live function moved to `crates/epsilon-tui/src/git.rs` with four tests, and the worktree pointer that used to render as the branch name `gitdir:` is now honestly reported as "no branch". | grep over `crates/`; `crates/epsilon-tui/src/ui/footer.rs` | — |

### Known and deliberately not claimed

- **The hidden-directory escape hatch was dead logic, and both walks now say so (M109).**
  `walk_inner` read `name.starts_with('.') && !ignore.is_ignored(..)`, and its comment
  promised that a `.gitignore` negation re-includes a hidden directory. It never could:
  `IgnoreSet::is_ignored` returns **false** both for "a rule ignored this and a later one
  re-included it" and for "no rule mentions it", so the clause changes the outcome only in
  the case where the next check skips the entry anyway. The tool walk's own test had already
  recorded the limitation ("NOT asserted, because it does not work yet"), so the code was
  honest in one place and false in two others — the inline comment and the public doc
  comment on `walk_files`. The clause is gone, both comments state the limitation with its
  reason, and the `@`-completion walk (`crates/epsilon-tui/src/at_files.rs`) matches rather
  than copying the illusion. Making the escape hatch real needs a matcher query that can
  tell "re-included" from "never mentioned"; `is_ignored` cannot, by construction.
- `epsilon-core::ignore` does not implement character classes (`[a-z]`), escaped
  metacharacters, or `core.excludesFile`. The module docs name all three.
- The header shows `0%%` in a **captured screen buffer** read through
  `ReadConsoleOutputCharacterW`; the rendered row is `0%`. The duplication is in
  the capture path, not the frame, and it is recorded here because it has not
  been proven to my own satisfaction on a real terminal — only explained.
- **The `@`-file completion now shares M99's answer — closed in M109.** It used to keep
  its own walker and hardcoded skip list, so `@` suggested files `.gitignore` ignores and
  the tools would refuse. `App::at_file_rows` now loads `epsilon_core::ignore` and the walk
  lives in `crates/epsilon-tui/src/at_files.rs`. This bullet said "queued, not done" until
  2026-09-21; the port re-check below caught it, which is the argument for re-reading the
  audit against the code rather than trusting it.
- **The port plan was written against a pi snapshot that is no longer on disk.**
  [`03-port-plan.md`](03-port-plan.md) cites `references/pi-0.84.2/` twenty-one times;
  only `pi-0.85.1` exists. Every row was re-checked against the current code and pi 0.85.1
  on 2026-09-21 — see
  [`evidence/port-gap-recheck-2026-09-21.md`](evidence/port-gap-recheck-2026-09-21.md),
  which lists the thirteen rows that still carry a gap, the two that closed since, and
  **twelve claims in `audit/03` and this file that are now factually wrong** (wrong paths,
  wrong line numbers, "no producer was found" for something that has one, a deleted Rust
  file, and a code comment that claims pi behaviour it does not match). Treat unverified
  line numbers in `03` as historical; the re-check is the current reading.

### Bookkeeping defects (not code)

These are the contradictions a fresh reader would trip over, and they are worth
fixing precisely because they make every other claim harder to trust:

- **Fixed 2026-09-21:** eight milestones (M96–M100, M102–M104) were committed
  without their dev log, and the timeline had no rows for M102–M104. Reconstructed
  from the commit diffs and messages; the one thing those logs cannot manufacture is
  evidence, and they say so — **none of the eight has a preserved battery log**, so
  each `## Verification` section reproduces the commit message's own measurements
  and names the gap. Their claims now rest on the workspace battery from this round.
- `audit/05-verification-log.md` is many batteries stale relative to the
  milestone logs, while `AGENTS.md` calls it the latest baseline. This file is the
  live baseline; that one is history.
- The M95 review record lived only in `%LOCALAPPDATA%\Temp`, outside the repo.
- `audit/10-tui-enhancement.md` names evidence logs whose names disagree with
  their contents (`tui-full-green.log` is a RED run).
- `epsilon --help` opened by promising "every flag, env var and file, in full" and
  omitted `epsilon doctor`. Fixed in `5fd12b5`, found by checking the README's
  commands against the binary rather than against the documentation — the check
  that catches this class.
- **The gate battery could not run at all under `bash` on this machine.** `bash`
  here is WSL: its own `$HOME`, its own PATH, no Windows `cargo`, so
  `bash scripts/gate.sh <label>` exited 127 in zero seconds and the log ended in
  `GATE FAIL` — which reads like five broken gates rather than one missing PATH
  entry. Fixed 2026-09-21 by adding `scripts/gate.ps1`, the native Windows battery;
  `gate.sh` keeps working for POSIX and CI, and now names the missing binary
  instead of failing anonymously.
- **A superseded `git stash` was dropped, with its content accounted for (2026-09-21).**
  `stash@{0}` ("WIP on main", 2026-09-20 11:02) held session-store work that never landed:
  an earlier `header_end`/`keep_end` (equivalent to M105's, which is better tested), a
  typed `UnknownLineShape` for an unknown final line (**that is F08 — it landed as M108,
  ported from here**), and three tests of which one carries leftover `eprintln!("PROBE …")`
  debugging and a nested store opened only to print bytes. Nothing unique remained, so the
  stash was dropped rather than left to read as unfinished work. It is still in the reflog
  for the usual window if that assessment is ever disputed.
- `agents/memory/next-hardening.md` still describes a baseline from before the
  TUI work. Its own header says the section is historical; the live baseline is
  this file.

## 3. The plan

Ordered by what unblocks what, not by what is interesting.

**Phase A — finish the TUI (owner priority, in flight).**
1. Migrate the four remaining list overlays onto `ui::overlay` *(in progress)*.
2. Split `app.rs`: `event.rs` for the loop, `keys/` per mode, leaving the draw
   path as the shell call plus overlays.
3. Delete `crate::theme`.
4. Check the ConPTY harness into the repo as a test target that renders frames
   and asserts on them, so a visual regression is a failing test.

**Phase B — close the port's behavioural gaps.**
5. G1 (skill loading) — smallest fix with the largest effect on the model's
   behaviour.
6. G5–G9 — each is a few lines; each currently makes a tool or a prompt promise
   something the harness does not do.
7. G9 and G10 get a regression test before the fix, because both are
   "sometimes" failures and a fix without a reproduction is a guess.

**Phase C — the P0 correctness work.**
8. G3/G4 checkpoint restore semantics, with an explicit tri-state
   (restored / absent / unreadable) rather than a boolean.
9. G2 resolved containment, which is a design task, not a patch.

**Phase D — teaching value and the long tail.**
10. Rewrite `README.md` and the docs map around the module tree that now exists.
11. `epsilon-ext` test coverage.
12. The remaining parity rows in `audit/03-port-plan.md`.

## 4. What this assessment does not claim

- No Linux or macOS execution has been performed. The Windows evidence is real
  and specific; the cross-platform claim is not tested.
- No live provider call was made during this assessment; cost and cache figures
  are exercised by tests, not by a paid session.
- The reference studies are reads, not ports: a divergence identified here is
  evidence that the code differs, and for two of them (D-2, D-5) the *running*
  consequence is inferred from two code paths rather than observed.
