# 01 — Current state

What is actually in this repository, and how much of it should be believed.
Counts in this document were taken on 2026-09-27 against commit `6c4063b`.

## Verdict

This is a real codebase, not a sketch. Roughly 83,000 lines of Rust across
twelve crates, with about 18,000 lines of tests, an offline test suite that runs
without a provider, and a terminal frontend that drives the real binary through
a real pseudo-terminal. The engineering is above average in places: the
provider-anchored context estimate in
`crates/epsilon-core/src/compact.rs:29-53` is a genuinely good idea, better than
anything in the reference harnesses; the permission engine's last-match-wins
rule is right and pure; the work that has gone into making numbers honest rather
than merely present is unusual and worth keeping.

It is also carrying three problems that need decisions, not more work:

1. **Documentation has drifted from the code, and the drift has been recorded as
   truth.** The agent memory in this repository named M135 as its next action
   when M178 was the last commit — forty-three milestones of drift. That is the
   failure mode this audit exists to stop.
2. **The terminal frontend has a per-frame cost that grows with the session, and
   a panic path that leaves the console unusable.** Both are in
   [04 — The TUI](04-tui.md).
3. **The three numbers in the header can each be wrong in a specific, known
   way**, and in one case the code that would make them right already exists and
   is not called. That is in [05](05-metrics.md).

## Size

Measured by line count, excluding `tests/` directories and `*_test.rs`:

| Crate | Source | Files | Tests | Test files |
|---|---:|---:|---:|---:|
| `epsilon-tui` | 26,437 | 49 | 0 | 0 |
| `epsilon-llm` | 19,506 | 24 | 1,270 | 4 |
| `epsilon-core` | 10,576 | 17 | 7,370 | 14 |
| `epsilon-tools` | 7,472 | 15 | 2,455 | 1 |
| `epsilon-cli` | 5,734 | 5 | 1,032 | 4 |
| `epsilon-session` | 4,533 | 7 | 4,334 | 2 |
| `epsilon-safety` | 2,398 | 9 | 833 | 4 |
| `epsilon-contracts` | 2,578 | 12 | 0 | 0 |
| `epsilon-lsp` | 1,300 | 7 | 137 | 1 |
| `epsilon-server` | 1,146 | 2 | 0 | 0 |
| `epsilon-ext` | 912 | 5 | 848 | 4 |
| `epsilon-mcp` | 538 | 3 | 72 | 1 |
| **Total** | **83,131** | **155** | **18,351** | **35** |

**Read that table before planning anything.** Three of twelve crates have no
`tests/` directory at all, and `epsilon-tui` — the largest crate, roughly a third
of the code, and the one with the reported defect — has no integration test
directory and relies entirely on inline tests plus one pseudo-terminal suite in
`crates/epsilon-cli/tests/terminal.rs`. Test-to-source ratio by crate ranges from
0.0 (`epsilon-tui`, `epsilon-contracts`, `epsilon-server`) to 0.96
(`epsilon-session`).

The eight largest source files:

| Lines | File |
|---:|---|
| 7,937 | `epsilon-llm/src/catalog/catalog_data.rs` — **generated**, not code |
| 3,233 | `epsilon-core/src/lib.rs` — the agent loop and agent state |
| 3,002 | `epsilon-cli/src/main.rs` |
| 2,447 | `epsilon-tui/src/state.rs` |
| 2,396 | `epsilon-session/src/store.rs` |
| 1,656 | `epsilon-llm/src/providers/mod.rs` |
| 1,535 | `epsilon-tui/src/picker.rs` |
| 1,147 | `epsilon-core/src/compact.rs` |

One of those eight is generated data and should not be counted as complexity.
The rest are large but none is a god object in the sense of having no
internal structure: `epsilon-core/src/lib.rs` is a crate root with a large
`Agent`, and a previous line of work had already begun splitting it.

## The dependency graph is real, and mostly matches its documentation

Read from all twelve manifests. Internal production edges:

```
contracts ──┬─> llm ──┐
            ├─> session ──> core ──┬─> safety ──> tools ──> (lsp) ──┐
            ├─> lsp ───────────────┼─> mcp ────────────────────────┤
            ├─> ext                 │                              │
            └────────────────────── > tui ─────────────────────────┤
                                   core ──> safety                 │
                                   llm, session, safety ──> tui ───┤
                                   core, safety ──> server ─────────┤
                                   all eleven ──> cli ──────────────┘
```

The four rules this repository states about its own layering **hold**: no
library depends on `epsilon-cli`; `epsilon-core` depends on exactly
`{contracts, llm, session}`; `epsilon-lsp` depends on `contracts` alone; there
is no cycle. `epsilon-safety` has a dev-dependency back on `epsilon-tools`,
which is legal and is how its integration tests get a real tool set.

Two edges are wrong in a way that matters, and one is wrong in a way that hides
a safety law — both in [03 — Defect ledger](03-defects.md), D1 through D3.

**The graph is not a layering, and no document says so.** `core` is a dependency
of five siblings; `tools` reaches down to `lsp`. That is a real structure — a set
of peers over a shared core, with the frontend crates on top — but
`docs/design/epsilon-architecture.md` draws it as a descending stack, and a
reader who trusts the picture will reason about new edges wrongly.

## What is genuinely good, and should not be lost in a restructure

These are not politenesses. Each is a decision that took real thought, and a
restructure that flattens them would make the codebase worse.

- **The context estimate is provider-anchored, not guessed.**
  `crates/epsilon-core/src/compact.rs:46-53` takes the provider's own count for
  the prefix and estimates only what was appended since. All four reference
  harnesses estimate, and all four are wrong by a factor of up to four in
  opposite directions on prose and on code. The header and the compaction
  trigger read the *same function*, so the number a user sees cannot flatter the
  loop that produced it. That property is worth more than the last ten
  milestones of TUI work.
- **Truncation is bounded and the bound is per-tool with a stated reason.**
  `read` pages, `bash` tails, `grep` caps lines and each says so. The
  references converge on the same need (Hermes `tools/tool_output_truncate.py`,
  opencode `src/tool/truncate.ts`, DSH's spill subsystem) and opencode's version
  — a single authority in the tool registry, with a self-aware hint — is better
  than what is here. See [06](06-references.md).
- **The permission engine is a pure function.** Matching denies win; otherwise
  the last matching rule decides. Property-testable, no I/O, no callbacks.
  opencode's `evaluate()` is the same shape.
- **The test suite runs offline.** A harness that can only be tested by spending
  money is a harness that is only tested when someone is paid to do it. The faux
  provider and the `SlowProvider` are the reason the loop is testable at all.
- **The terminal test drives the real binary through a real pty**, and answers
  the pty's cursor-position query — a detail that costs an afternoon to learn and
  whose absence makes the harness look like a program that renders nothing.

## What the previous audit got wrong, or left unverified

Kept because leaving it in place is the actual risk.

| Earlier claim | Status |
|---|---|
| "`agents/memory/README.md` — next safe action: M135 is in flight" | **False.** M178 was the last commit. The note had not been updated for forty-three milestones. |
| "The typing-freeze report was two real defects, both already fixed and both measured" | **Unverified, and now doubtful.** The measurement that established it (`audit/evidence/console-probe-2026-09-21.md`) typed into an *idle* TUI against the *faux* provider, and the milestone that added the shell tests says so in its own `Limits` (`docs/milestones/M178-the-shell-in-the-suite.md:107-109`). The reported symptom involves a run in flight. See [04](04-tui.md). |
| "12 workspace crates" (audit/12) and "Twelve crates" (README) | **True**, and the audit's correction of an earlier "11" was right. |
| "`docs/design/epsilon-architecture.md` maps the real edges and the execution paths" | **Half false.** The edge table is exactly right. Five execution-path claims are false and four are stale — [03](03-defects.md) D4 lists them. |
| "A test the battery skips is a test that rots" | **True, and the fix was in flight uncommitted** at the time of this audit: an M179 change making `crates/epsilon-cli/tests/terminal.rs` find its binary through `CARGO_BIN_EXE_epsilon` instead of a hand-set `EPSILON_BIN`. It is correct and this audit keeps it. |

## What this audit could not establish

Stated plainly, because an audit that hides its gaps is worse than no audit.

- **Whether the TUI freeze reproduces.** [04](04-tui.md) names the mechanisms
  that would cause it and the experiment that distinguishes them, but the
  measurement needs a built binary in a real conhost and had not been taken when
  this document was written. It is taken during the work, and the result —
  including "it did not reproduce" — is recorded there.
- **Linux and macOS behaviour.** Nothing here was executed on either. The
  bubblewrap path, the `nix` dependency, and the POSIX terminal paths are
  unverified beyond compiling.
- **Real-provider behaviour.** No live call was made. Every provider adapter's
  fidelity to its wire format is read from the source, not observed. The bundled
  model catalog is a generated snapshot of `models.dev` taken 2026-09-21; it will
  drift, and the CI job that checks for drift is the only thing watching.
- **Throughput.** No benchmark beyond the TUI frame benchmark exists, and that
  one uses ratatui's in-memory `TestBackend`, so it measures draw cost and never
  the console write — which is the part that differs most between a developer's
  Windows Terminal and a real `conhost` window.
