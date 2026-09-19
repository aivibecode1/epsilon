# M78 — per-call sandbox modes: one mode per call, resolved before anything spawns

**Scope:** port-plan Tier 4 item 15 (the second half of roadmap Phase 5 item 3), DSH
`packages/sandbox/sandbox/src/escalation.ts:28,51,71,85,157`.

## The problem

`bwrap_argv` mounted the workspace writable unconditionally, so a review task that only ever reads
had no way to *say* so and no way to have it enforced. There was one policy per registry and a
`--sandbox auto|off` switch — a session-wide choice with nothing in between, and no vocabulary for a
call that wants its own narrower boundary.

## What landed

**1. The mode vocabulary** (`epsilon-safety/src/escalation.rs`), a three-rung ladder:

```rust
pub enum SandboxMode { ReadOnly, WorkspaceWrite, DangerFullAccess }
```

with `wider()` as an explicit table (`read-only → {workspace-write, danger-full-access}`,
`workspace-write → {danger-full-access}`), `is_strictly_wider_than` read off that table, and
`ALL` as the closed advertisement vocabulary — not narrowed to the modes above some default, because
a session sitting low on the ladder must still be able to name what it wants (DSH
`ESCALATION_TARGETS`). `parse` refuses unknown names, so a typo can be neither the narrowest nor the
widest mode by accident.

**2. The paired arguments.** A call may carry `sandbox_permissions` **and** `justification`; they
travel together or not at all, and the justification must be a sentence. Half a request is an error,
never a default — exactly the pairing a JSON schema cannot express, which is why DSH validates it at
execution time.

**3. Enforced at the last moment before spawn.** `ConfinedCommand` carries the mode; the bubblewrap
backend mounts the workspace `--ro-bind` for `ReadOnly` and `--bind` otherwise; both backends
**refuse** `DangerFullAccess` outright (`"no confinement" is not a confinement`) rather than running
with their own limits and calling it full access.

**4. The session gained the middle rung:** `--sandbox read-only` attaches a backend and starts every
call at `ReadOnly`; a call may then ask for `workspace-write` and is told that widening is a user
decision. A call may always *narrow* itself — asking for less is a gain in safety, so it needs no
approval.

**5. Nothing claims a boundary it doesn't have.** With confinement off, a read-only request is
refused rather than honored, because the command would run unconfined whatever it said. On Windows a
read-only call still reports `enforcement: partial — not confined: filesystem` from M77, which is the
honest answer.

## Key decisions

- **The mode is per-call truth, not a schema enum.** A schema is registry-global; what a *call* may
  do depends on the session and on consent. That is why the ladder is an execution check.
- **Widening is refused, not prompted — and that is a deliberate limitation.** DSH's ladder ends in an
  approval ask. Epsilon's per-call consent channel would have to be a harness-owned field on
  `ToolContext` (args belong to the model, so a grant written there would be forgeable), and the mode
  type would have to move to `epsilon-contracts` for the context to hold it — an ADR-level change,
  not a drive-by. What the user consents to today is the **session** mode, which is a real consent
  and a real boundary; the refusal text names the lever (`--sandbox off`) so the model can surface a
  truthful ask. Recorded as the next item rather than faked.
- **Table, not comparison.** `is_strictly_wider_than` reads the ladder instead of `self > current`:
  a new variant makes the match non-exhaustive and will not compile until someone decides where it
  sits, while an ordering comparison would widen silently. A test ties the table to the declared
  order so the two cannot drift.
- **The refinement type is the test:** `Partial { missing: [] }` is unrepresentable (M77), the ladder
  is a table the compiler checks for completeness, and an unknown mode name is an error — three ways
  this design says "no silent middle".
- **`DangerFullAccess` never reaches a backend.** It is reachable only as the session choice
  (`--sandbox off`), i.e. where a human decided it.

## Verification

Six mutations, six discriminations:

| Mutation | Test that fails |
|---|---|
| the ladder never widens (`is_strictly_wider_than → false`) | `the_ladder_strictly_widens_and_never_wraps` |
| the escape-hatch refusal removed | `a_call_asking_for_danger_full_access_is_refused_before_anything_runs` (asserts the marker file does **not** exist) |
| half a request accepted | `the_paired_arguments_must_travel_together` |
| read-only mounts the workspace writable | `read_only_mounts_the_workspace_read_only` |
| unconfined path accepts a narrowing claim | `with_confinement_off_a_narrowing_request_is_refused_rather_than_lied_about` |
| the CLI drops the rung | `the_binary_accepts_the_read_only_sandbox_rung_and_refuses_invention` |

Nine new tests: five in `epsilon-safety` (ladder shape, table-versus-order, narrowing, argument
pairing, typo rejection, plus the read-only argv and the two backend refusals), five at the tool
boundary (narrowing honored with `details.mode`, the refusal proving nothing ran, widening refused
with the hint, paired arguments, and the confinement-off refusal), one binary-level.

A test-caught bug: `a_call_may_narrow_its_own_sandbox_mode_to_read_only` first ran against a tool
with **no** backend attached and failed — because the new confinement-off rule correctly refuses a
read-only claim. The premise was wrong, not the rule: the test now attaches a backend and asserts the
refusal path where none exists.

Gate: `scripts/gate.sh m78b` — fmt clean, clippy zero warnings, **739 tests, 0 failed**, demo
completes, `real`-feature build compiles (16 new tests since M77's 723).

The first run was a GATE FAIL with all 739 tests *passing*: clippy exit 101 on
`field assignment outside of initializer for an instance created with Default::default()` in one of
the new tests. `cargo check --workspace --all-targets` was clean, because this is a clippy lint and
not a rustc one — which is precisely what the `-D warnings` hardening after M69 is for. The test was
rebuilt, and the whole battery re-run rather than just the step that failed: a gate claim covers the
battery.
