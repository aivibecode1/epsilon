# Epsilon — audit

An independent assessment of this repository, written on 2026-09-28 from the code, the
four reference harnesses, and measurements taken on this machine.

It does not inherit any earlier verdict. **Where it contradicts one, it says so and
shows the command** — there are two retractions in here, and both are more useful than
the findings they replace.

## Read this first

The documents in [`history/`](history/) are earlier assessments. They are kept because
the reasoning in some of them is still worth reading, and because deleting a record of
what someone believed is its own kind of dishonesty. **They are not current.** Three are
marked where this audit disproves them:

| Superseded document | What this audit found |
|---|---|
| [`history/13`](history/13-tui-superseded.md) — the TUI | Its root cause (a silent permission dialog) is **wrong**; typing and the turn were measured working. See [04](04-tui.md). |
| [`history/14`](history/14-current-state-superseded.md) — current state | Replaced by [01](01-current-state.md). |
| [`03-defects.md` D2](03-defects.md) | Retracted. The proposed fix would have **broken the build**. |

## The audit

| # | Document | What it answers |
|---|---|---|
| [01 — Current state](01-current-state.md) | What exists, how big it is, what was measured and how | "Is this a real codebase, and do I trust what I am reading?" |
| [02 — Port fidelity](02-port-fidelity.md) | How much of `pi-agent` is faithfully ported, and where it is not | "Is the port finished?" |
| [03 — Defect ledger](03-defects.md) | Every confirmed defect, ranked, with the line that has to change | "What is broken, and what do I fix first?" |
| [04 — The TUI](04-tui.md) | The Windows typing-freeze: what I measured, what it rules out, what remains | "Why is the TUI frozen, and how will I know it is fixed?" |
| [05 — Context, cache, cost](05-metrics.md) | Whether the three numbers in the header are true, and what makes them untrue | "Can I trust the numbers the user is shown?" |
| [06 — What to take from the references](06-references.md) | The strongest ideas in the four references — and the traps | "What should this become?" |
| [07 — Plan](07-plan.md) | The ordered work, with dependencies and what each step proves | "What happens next, in what order?" |

## The three findings that matter most

**1. The repository cannot currently reproduce its own worst bug.** The faux provider
emits an entire turn's events instantly (`crates/epsilon-llm/src/faux.rs:145-148`).
Every TUI test and probe this project has ever run therefore photographed a turn that
was over in microseconds — never a run streaming over seconds while the user types, which
is the state the owner reported. `SlowProvider` cannot fill the gap: it delays a turn's
*start*, not its *duration*, and nothing wires it to the TUI. Details in
[04](04-tui.md), fix in [07](07-plan.md) Phase 1.

**2. Subagent tokens are never counted, and that is about money.** The `task` tool
produces usage, `exec.rs:736` carries it into the tool result, and the TUI never reads
it. A session that fans out under-reports its cost by an unbounded multiple with no
marker. Verified end to end; see [03](03-defects.md) D17.

**3. The strongest asset here is the writing, and the weakest is the hygiene.** The
prose in `docs/qa/` and the milestone logs is genuinely excellent — a project recording
its own mistakes precisely. What fails is structural: ~155 broken links, 11 MB of gate
logs in git, a README whose own honesty disclaimer is a dead link, and six of eight
skills that belong to a different project. See [01](01-current-state.md).

## How this audit was produced

**Sources of truth, in order:** the code; a command run in this audit; `git log`; the
reference harnesses. Documentation is a *claim about* the code, never evidence for it.

**Evidence labels** are defined in [01](01-current-state.md): **Measured**, **Read**,
**Inferred**, **Unverified**.

**Reviews were delegated, then re-checked.** Four large read-only studies were run in
parallel — TUI root cause, port fidelity against `pi-0.85.1`, metrics accuracy, and
layout and documentation. **Every delegated finding was re-verified against the source
before it was written down**, and two did not survive: the "12 vs 11 crates"
contradiction was my own miscount (`epsilon-safety` was missing from my `ls`), and
defect D2 was wrong in a way that would have broken the build. Both are recorded rather
than quietly dropped, because a delegated finding that turns out to be wrong is
information about the method.

**What was not done here:** no live provider call was made, no credential store was
read, no Linux or macOS execution was performed, and **the full test suite was not run**.
Nothing in Phase 0 of the plan changes product code, so the suite state should be
unchanged from HEAD — but that is an inference about an unrun suite, and it is stated as
one rather than assumed.
