# M56 — Per-delegation model override

Delegation was already a mechanism (M53) with a control surface (M54). What it
lacked was a cost lever: a child always ran on whatever the agent type said, or
on the parent's own model. Scouting a wide area, sweeping a mechanical rename,
or reading twenty files does not need the session's frontier model — and the
model doing the delegating is the one who knows when a cheap child is enough.

opencode has the same lever through agent configuration (`next.model ?? parent
model`, with the resolved model reported in the task metadata); pi has no task
tool at all, so opencode is the reference here. Epsilon now has both halves:

**The call can choose** (`task(model: "deepseek-flash")`). `TaskRequest` gained
`model`, the tool schema advertises it, and the description teaches it —
including why the *result* must be read before assuming anything.

**Precedence is explicit**: `agent-def model` → `request model` → inherited
parent model. Operator configuration outranks a call (a def's model is the
type's contract), the call's request outranks inheritance, and the tool result
plus the registry always name the model that actually ran — so a delegation can
never silently diverge from what was asked for.

**Refusals instead of guesses.** `resolve_task_model` is deliberately stricter
than `resolve_model`'s partial rule: exact, `provider/id`, and case-insensitive
matches are accepted; a partial match is *offered* ("only partially matches
`deepseek-v4-pro` — pass the full id") instead of used, because a delegated run
switching models on a fuzzy guess is a cost and quality surprise nobody asked
for. An unknown id fails before any child is built, with the registry left
untouched (test-pinned).

**Everything that reports a task now reports the model**: the tool envelope
(`<model>…</model>`), the tool `details`, the snapshot contract, the TUI's
`/tasks` (`task <id> · running · @deepseek-flash · …`), and the background
completion notice (`<task id="…" state="completed" description="…"
model="deepseek-flash">`), which is what the *parent model* reads when a
detached child finishes.

## Key decisions

- **Def beats request.** opencode's agent config is authoritative for a type;
  a call that wants a different model for that type should change the type (or
  use `default`). The alternative — a call overriding the operator — makes a
  configured fleet of scouts unauditable.
- **Strict resolution, no silent substitution.** The partial-match rule that
  serves humans at the picker is wrong for a machine-to-machine delegation:
  a wrong model is worse than an error, and the error is one line to fix.
- **The model travels with the entry**, not with the run: a resumed child keeps
  the model it was created with (documented in the description: `model` applies
  when a task starts), and a status report can say what it runs on even while
  it is mid-flight.
- **A drive-by fix in the same area**: the background path used to build its
  envelope with an empty subagent type (`summarize_task_output("", …)`), so a
  detached child with no textual output reported `[subagent : …]`. It now
  receives the real type — and the model.

## Gotchas

- Three test doubles construct `TaskSnapshot`/`TaskRequest` (`RecordingHost`
  twice, the scenario helpers once): adding a field to a contracts struct is
  cheap in the library and a small tax on every double. Keeping the `req(...)`
  helper's signature and adding `req_with(...)` kept ten existing scenario
  calls untouched.
- The notice assertion must read the notice as a *user* message: the model
  attribute lives inside the envelope text the parent model reads, not in any
  structured event field, so the test extracts it the same way the M53 notice
  test does.

## Verification

- `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets`
  zero diagnostics.
- `cargo test --workspace`: **422 passed, 0 failed** (416 before; +4 core
  scenarios, +2 tool tests). `--features real` lane: 125 passed, 0 failed.
- New coverage: four core scenarios (request honored + reported, unknown id and
  partial id refused with nothing registered, def outranks the request, the
  background notice names the model), two tool tests (the parameter reaches the
  host and the envelope, the schema advertises it), and the TUI `/tasks` view
  naming the child's model.
- `cargo run -q -p epsilon-cli -- --demo` completes.
