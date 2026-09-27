# Epsilon — audit

An independent assessment of this repository, written on 2026-09-27 from the code,
the four reference harnesses, and measurements taken on this machine. It does not
inherit any earlier verdict; where it contradicts one, it says so and shows the
evidence.

## Read this first

The documents in [`history/`](history/) are earlier assessments. They are kept
because the reasoning in them is still worth reading and because deleting a
record of what someone believed is its own kind of dishonesty. They are **not**
current. Two of them make claims this audit disproves; those are marked where
they are wrong.

## The audit

| # | Document | What it answers |
|---|---|---|
| [01 — Current state](01-current-state.md) | What exists, how big it is, what was measured and how | "Is this a real codebase, and do I trust what I am reading?" |
| [02 — Port fidelity](02-port-fidelity.md) | How much of `pi-agent` is faithfully ported, and where it is not | "Is the port finished?" |
| [03 — Defect ledger](03-defects.md) | Every confirmed defect, ranked, with the line that has to change | "What is broken, and what do I fix first?" |
| [04 — The TUI](04-tui.md) | The Windows typing-freeze investigation, the per-frame costs, and how to prove a fix | "Why is the TUI frozen, and how will I know it is fixed?" |
| [05 — Context, cache, cost](05-metrics.md) | Whether the three numbers in the header are true, and what makes them untrue | "Can I trust the numbers the user is shown?" |
| [06 — What to take from the references](06-references.md) | The strongest architectural ideas in the four reference harnesses, and which are traps | "What should this become?" |
| [07 — Plan](07-plan.md) | The ordered work, with dependencies and what each step proves | "What happens next, in what order?" |

## How this audit was produced

**Sources of truth, in order:** the code; a command run in this audit; `git log`;
the reference harnesses in `references/`. Documentation is a *claim about* the
code, never evidence for it.

**Evidence labels used throughout:**

- **Measured** — observed from a command in this audit. The command is named.
- **Read** — the control flow and its callers were inspected. Establishes what the
  code does, not what a user sees.
- **Inferred** — a conditional consequence. The trigger and the test that would
  settle it are both stated.
- **Unverified** — no adequate evidence yet. Do not treat as a capability.

**What was not done here:** no live provider call was made, no credential store
was read, no Linux or macOS execution was performed, no benchmark suite was run
beyond what is named. The tests that exist were run; the claims that they cover
the TUI under a real provider were not verified, and [04](04-tui.md) explains
why that gap matters.

**Reviews were delegated, not assumed.** Large read-only studies were run as
independent passes over the code and the references, and their findings were
re-checked against the source before being written down here. A delegated
finding that did not survive re-checking is not in this audit.
