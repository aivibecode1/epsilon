# 07 — Plan

Ordered work, with what each step proves and what it depends on. The ordering rule is
simple: **build the ability to verify before making the change you want verified.**

Every item names the evidence that will show it is done. An item whose evidence is "the
tests still pass" is not finished; that is what a broken change looks like too.

## The ordering, and why

```
  Phase 0 ── honesty        fix what misleads a reader        (hours, zero risk)
     │
  Phase 1 ── verify first   the TUI harness                   (the gate for everything)
     │
  Phase 2 ── the TUI        now measurable, so now fixable
     │
  Phase 3 ── the numbers    metrics accuracy
     │
  Phase 4 ── the port       finish the cheap parity wins
     │
  Phase 5 ── the structure  split the monolith, dedupe providers
```

Phase 0 exists because two documents currently tell a reader something false, and one
of them is the README's own honesty disclaimer. That is a worse problem than any defect
in this list.

## Phase 0 — honesty (do first, it is nearly free)

| # | Work | Evidence it is done |
|---|---|---|
| 0.1 | Fix the two dead README links: `audit/11-current-assessment.md` and `audit/06-reference-comparison.md` | Both resolve; `grep` finds no `audit/NN-*.md` link that is not in `history/` |
| 0.2 | Repair `docs/README.md` — 18 orphaned table rows sit above the `# Documentation` heading with no header row | The file starts with its heading |
| 0.3 | Mark or delete `docs/qa/03-partial-vs-full-port.md` | No doc claims tools/permissions/compaction/MCP are missing |
| 0.4 | `git rm --cached audit/evidence/*.log`, add the ignore rule | 173 files and ~11 MB out of git; `.md`/`.patch` evidence retained |
| 0.5 | Delete the 6 `ponytail-*` skills and the 4 `.tmp-*` files | `benchmarks/` is cited by `ponytail-gain` and does not exist; the `ponytail:` comments are prose, not the format `ponytail-debt` parses |
| 0.6 | Promote `epsilon-architecture.draft.md`, retire the 172-line live one | One architecture document, and the draft's "what this does not establish" survives |
| 0.7 | Fix the ~155 broken links repo-wide | A link checker passes |

**0.5 deserves its reasoning.** The `ponytail-*` family is five skills (of eight) that
belong to a different project: one renders a benchmark scoreboard citing a `benchmarks/`
directory this repository does not have; one greps for `ponytail:` comments in a
`ceiling / upgrade-path` format that the 19 existing comments do not use; and all of them
ship slash-command mode-switching UX that belongs to a chat plugin. They are the kind of
thing that reads as rigor and functions as noise. Delete them.

## Phase 1 — the harness (nothing after this is verifiable without it)

| # | Work | Evidence |
|---|---|---|
| 1.1 | A provider that streams over a **controllable duration**, not just a pre-stream delay. `SlowProvider` (`core/src/testing.rs:71`) cannot do this. | A test drives a 3-second turn and asserts events arrived spread over time |
| 1.2 | Wire it to the TUI — a flag the console probe and the terminal test can both use | `--script` reaches the TUI today; this reaches it with *timing* |
| 1.3 | A conhost test that types **while a turn is in flight** | The character is visible, **or** the screen says why it was not |
| 1.4 | The invariant, stated as a test name: *nothing happening must never be a reachable UI state* | It fails against a build that swallows keys |

This is the phase that makes the rest of the audit's claims checkable. [04](04-tui.md)
argues it at length: **no test or probe has ever run this TUI while a turn was genuinely
in flight**, because the faux provider emits every event instantly
(`llm/src/faux.rs:145-148`). Every green TUI result this repository holds comes from a
provider that cannot reproduce the bug.

## Phase 2 — the TUI

| # | Work | Depends on | Evidence |
|---|---|---|---|
| 2.1 | Fix T1: the per-frame transcript deep clone. The tripwire test already fails. | — | `context_mirror_cost_does_not_track_transcript_length` passes |
| 2.2 | Fix T3: `restale_finished_cards` is O(session) per *event*; make it a set and move it off the per-event path | — | A benchmark shows cost flat in transcript length |
| 2.3 | Fix T4: panic hook + `Drop` guard so a panic restores the terminal | — | A panicking path leaves a usable shell |
| 2.4 | Fix the stale `tui.lock`: probe the recorded pid, or shorten the steal window | — | A killed TUI does not block the next launch for 30 s |
| 2.5 | **Re-measure the freeze with 1.x in place** | 1.x | A measurement, not an inference. This is where the real cause gets named. |
| 2.6 | R3: make the `bash` default proportionate; composer visibly inert under a modal; no key discarded without feedback | 2.5 | A test asserts feedback for an unrecognised key |
| 2.7 | Reduce the per-turn tool set (R2) | — | Fewer schema tokens in every request |

**2.5 is the honest centre of this plan.** Steps 2.1–2.4 are real defects worth fixing
regardless. None of them is *proven* to be the reported freeze, and until 1.x exists that
cannot be established. I would rather say that plainly than reorder the plan to imply a
confidence the evidence does not support.

## Phase 3 — the numbers

Ranked by [05](05-metrics.md), largest user impact first.

| # | Work | Evidence |
|---|---|---|
| 3.1 | **$1** — count delegated subagent tokens. `task.rs:231,281` produces them, `exec.rs:736` carries them, and `state.rs:1022-1061` drops them. | A test that fans out and asserts the cost rose by the child's usage |
| 3.2 | **$2** — show the compaction summary's bill live, not only after `/resume` | Cost does not jump when a session is reopened |
| 3.3 | **K1** — a cache-capability flag on `Usage`, so "cannot report" is not "measured zero" | A faux/ollama run prints `CH n/a`, not `CH 0%` |
| 3.4 | **C2** — include the system prompt and tool schemas in the no-anchor fallback | `ctx` after a compaction matches the next real request |
| 3.5 | **C1** — correct or live-source the context window; it is wrong by 6× for some DeepSeek rows | `doctor` names the discrepancy |
| 3.6 | **$3** — recover usage from a stream that dies before the final chunk | A failed turn still reports its prompt tokens |
| 3.7 | **K2** — show a session-rate beside the per-call rate | Both visible |

3.1 is one reader in one place and it is about money. Do it first regardless of
everything else on this list.

## Phase 4 — finish the port

| # | Work | Evidence |
|---|---|---|
| 4.1 | ~~**R1** — make `/tree` navigate.~~ **DONE 2026-09-28** (`7489c00`): navigation was already wired; the defect was that the transcript did not follow the leaf. Re-projection added, negative control confirmed | Leaf moved ⇒ the visible conversation is that branch |
| 4.2 | ~~Prompt templates (`/name` expansion).~~ **DONE** (`3902461` engine, `d9efe9f` TUI wiring). Full bash-style substitution grammar, ported from `prompt-templates.ts`. Control: removing the submit-site call fails the wiring test while the three direct-call tests pass | ✅ |
| 4.3 | ~~Decide the extension surface.~~ **DONE `e9d5b94`** — ADR-019. Follow-ups deliberately not decided: the 27 UI methods, 7 registration APIs, message injection, provider registration | ✅ |
| 4.4 | Refresh the catalog and add a drift check | CI fails when the snapshot ages past its window |
| 4.5a | ~~Bidirectional event frames.~~ **DONE `ece3af4`** — `interpose` / `interpose_result` over a demultiplexing reader task. Control: forwarding an event during a slow invoke took 9.74 s before, ms after | ✅ |
| 4.5b | ~~Wire `ToolHooks`.~~ **DONE `9f73a5f` (before) / `2fb5219` (after)** — a child blocks a call and rewrites the result. Controls fail for the reason they name | ✅ |
| 4.5c | ~~Wire `TurnHooks`.~~ **DONE `570819e`**, and a **partial port by decision**: `transform_context`/`convert_to_llm` are pass-through, so extensions cannot reshape the prompt. Reasoning in [`02-port-fidelity.md`](02-port-fidelity.md) | ✅ (partial, deliberately) |
| 4.6 | **Real extension** — the *documentation* half is **DONE** (`cfdf94c`): `docs/user/extensions.md` with anchored drift tests over it. The **real extension** half is still open — every claim in 4.5a–c is proven against `ext-fixture`, a binary in this repository, so whether an author could actually use this is untested. Next concrete step: write one that is not a test binary | A documented extension works without reading the source |

**4.4 is small and prevents a class of bug that has already happened**: the
2026-09-21 snapshot is stale and silently wrong for some rows.

**4.5 is done, and 4.6 exists because "done" is not the same as "usable."** Four contracts
seams are now reachable from a real child process, every one proved against a live process
with a negative control that fails for the reason it names. But every proof used
`ext-fixture` — a binary in this repository. Nothing here demonstrates that a third party
could write an extension from the documentation, because the protocol's request/response
frames, the decision shapes, and the failure policy are currently described only in a
milestone log. That is the last honest gap in this area, and it is cheap.

## Phase 5 — structure (only after the above is green)

| # | Work | Evidence |
|---|---|---|
| 5.1 | Split `crates/epsilon-core/src/lib.rs` (3,415 lines / 155 KB, with 16 sibling modules at 1–46 KB) into `agent.rs` / `loop_.rs` / `schedule.rs` / `turn.rs`, following the M161–M166 pattern already proven in `epsilon-tui` | No behaviour change; full suite green |
| 5.2 | Dedupe the 9 provider files that each hand-copy the same 7 methods | One place to change a provider's boilerplate |
| 5.3 | Hoist `now_ms` (7 copies) and `resource_key` (6 copies) into `epsilon-contracts` | One definition each |
| 5.4 | Split the 19 ADRs out of the 95 KB `epsilon-decisions.md` into `docs/adr/` | One decision per file |
| 5.5 | Archive `docs/milestones/` (171 files) to a branch, keeping the ~25 that document a design decision or a non-obvious root cause | The remaining set is worth reading |
| 5.6 | Add `rust-toolchain.toml` (MSRV 1.88 is declared but unpinned) and a 20-line `CONTRIBUTING.md` | A fresh clone builds on the declared toolchain |

**Do not merge crates.** The dependency graph is a clean DAG and ADR-014's "brick earns
existence" rule is holding. The layout is correct; the problem is file size inside
`epsilon-core`, not crate boundaries.

## Verification discipline

This is the part that decides whether any of the above is real.

- **Every claim names the command that produced it.** A log for bytes that are no longer
  in the commit proves nothing; freeze the candidate, run against that.
- **When a slice cannot have a pre-fix RED** — its API did not exist before — run a
  *negative control*: revert one fix in a disposable copy and show the test naming that
  property fails for the reason it names. That is how 1.4 and 3.1 get their evidence.
- **Fix root causes, not symptoms.** If a TUI fix makes a symptom go away without a test
  that names the property, it is not done.
- **Re-run the real-console probe after any TUI change.** In-process `TestBackend` frames
  cannot see whether the binary reached a terminal, which is where this bug lives.
- **Wipe the build cache after every round.** `D:/eps-target`, `CARGO_BUILD_JOBS=1`, and
  `df -h /c` at the end. C: is at 93%.

## What this plan deliberately does not do

- It does not re-verify claims from [`history/`](history/). Those are kept as a record of
  what someone believed, not as authorization.
- It does not merge crates, add a plugin system, or build a package manager. See
  [06](06-references.md) for why each of those is a trap at this stage.
- It does not treat the reported freeze as solved. It builds the ability to solve it,
  fixes the defects that are provable today, and then measures.
