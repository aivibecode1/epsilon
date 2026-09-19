# M77 — sandbox truthfulness: a boundary report is a fact, not a paragraph

**Scope:** port-plan Tier 4 items 14 (enforcement completeness) and 16 (the three-part runner
failure predicate), plus roadmap Phase 5 item 3.

## The problem

`Sandbox::run` returned an outcome that said nothing about the *quality* of the confinement. On
Windows that left the real caveat — job objects bound memory, process count and CPU, and restrict
nothing about what a process reads or writes — as a paragraph in an audit report. A paragraph is not
something a caller can check, and the sandbox's actual behaviour and the doctor's description of it
were two independent statements that could drift apart without anyone noticing.

Two failure classes, one theme: **do not say more than you can support.**

## What landed

**1. `Enforcement` on every confined outcome** (`epsilon-safety`):

```rust
pub enum Enforcement {
    Full,
    Partial { missing: Vec<&'static str> },   // "filesystem", "network", "memory", …
}
```

`Enforcement::from_missing` makes `Partial { missing: [] }` unrepresentable — an empty list is
`Full` by construction, so a report never has to invent a sentence for a state that means nothing.

**2. `Sandbox::capabilities()`**, a new trait method: what a backend can confine *at all* on this
platform, independent of policy. Each backend answers for itself:

| Backend | Claim |
|---|---|
| Windows job objects | `partial — not confined: filesystem, network` |
| bubblewrap (Linux) | `partial — not confined: memory, process tree, wall clock` |
| null (fail-closed) | `partial — not confined: filesystem, network, memory, process tree` |

The bubblewrap answer is itself a finding: that backend does **not** apply the policy's wall-clock
limit (only the job-object backend does), and the earlier code said nothing about it — `_policy` was
simply unused. It now says so, and the behavioural gap is recorded as open in the plan rather than
silently inherited: closing it needs a Linux run to verify, and this machine cannot give me one.

**3. The three-part predicate** (`epsilon-tools`, `CommandVerdict`):

| Question | Answer |
|---|---|
| Did the *runner* fail? | it never produces an outcome — `SandboxError` is the only source of a confinement verdict |
| Was the exit code in the call's allowed set? | `Exited(code)` — a *command* outcome |
| Is this a confinement refusal? | only when the backend refused; never inferred from output text |

`details` now carries `verdict` (`success` / `exited:7` / `signalled`) and `enforcement` on **both**
the confined and the switched-off path, so a caller branches on one vocabulary.

**4. `epsilon doctor` asks the backend** instead of asserting a `cfg` string:

```
sandbox
  confinement        Auto
  platform backend   windows-job-object (available) — cannot confine: filesystem, network
```

## Key decisions

- **The outcome reports the backend's envelope, not a per-field audit of the policy.** A field-by-
  field audit is what per-call modes (plan item 15) will need; an envelope is what a caller has to
  know today, and it is honest at every field it names.
- **A refusal is not an outcome.** `SandboxError` stays the single source of "the boundary stopped
  this", which is precisely what makes the third question answerable — the alternative (scanning
  output text for a fatal signature) is a heuristic that mislabels `rg` exiting 1.
- **`Partial` names dimensions, never a boolean.** "Partial" alone invites the reader to guess what
  is missing; the list is the part that can be acted on.
- **The Linux wall-clock gap is declared, not quietly fixed.** Writing untestable Linux code and
  calling it done would repeat the mistake this milestone is about; the caveat is now checkable and
  the fix is in the plan.
- `CommandVerdict` is exported from `epsilon-tools` rather than left unreachable inside a private
  module — a vocabulary nobody can import is not a vocabulary (and the compiler said so first).

## Verification

| Mutation | Result |
|---|---|
| the Windows backend claims `Full` | `the_job_object_backend_does_not_claim_to_confine_content` fails |
| outcomes always claim `Full` | `the_result_reports_how_completely_the_boundary_held` fails |
| non-zero exits read as success | `a_non_zero_exit_is_a_command_outcome_never_a_confinement_one` fails |
| `doctor` hardcodes a claim | `the_sandbox_line_comes_from_the_backend_not_from_this_file` fails |

New tests: 4 in `epsilon-safety` (the null backend's honest zero, the Windows caveat as a value, and
a portability test that whatever backend the platform picks states a coherent envelope), 3 in
`epsilon-tools`' unit module (the predicate's three answers, and that an empty `Partial` cannot be
built), 3 at the tool boundary in `builtin_tools.rs` (a real `exit 7` through the bash tool carries
`verdict: exited:7` and never the `[confinement]` marker; a refused call is an error with no verdict;
a confined run names every dimension the platform cannot confine), and 1 in `doctor` tying the
reported line to the backend's own claim.

Gate: `scripts/gate.sh m77b` — fmt clean, clippy zero warnings, **723 tests, 0 failed**, demo
completes, `real`-feature build compiles.

Two honest notes about that gate. The first run (`m77`) died in the *test* phase with
`link.exe ... exit code 0xc0000142` while linking `cache_attribution` — a Windows process-init
failure, not a compile diagnostic: the same binary linked in the previous milestone's gate, free RAM
was 8.5 GB of 16.6, disk was 57 GB free, and no stray compilers were running. The re-run linked and
passed with zero occurrences of that code, so it was environmental; the battery was not weakened and
nothing was excluded. Second, the count in the prose here is 723 where the arithmetic in my head
said 724 — measured, not predicted, and the docs were corrected to the measurement (again).

## Plan status

Items 14 and 16 are **ported**. Tier 4 still has item 15 (per-call mode vocabulary:
`read-only` / `workspace-write` / `danger-full-access` with an escalation table and approval before
execution), which is the larger half of roadmap item 3, plus the newly recorded Linux wall-clock
gap.
