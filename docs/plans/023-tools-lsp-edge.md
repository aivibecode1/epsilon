# 023 — the `epsilon-tools` → `epsilon-lsp` edge, removed by inversion (A8)

**Ledger row:** A8. **Status:** specified, not started. **Estimate:** one slice, ~1 hour warm, plus a
battery. **Precedent:** `ToolContext::task_host` and `ToolContext::goal_host` — the pattern this
repository already uses twice for exactly this shape.

## The finding, read rather than counted

`crates/epsilon-tools/Cargo.toml` carries a direct `epsilon-lsp` dependency. Its **whole** purpose is
one match arm:

```rust
// epsilon-tools/src/lib.rs
pub enum Lsp { Off, Auto }                 // :82  — the policy, decided by the caller

pub fn register_builtins_with(registry: &mut ToolRegistry, confinement: &Confinement, lsp: Lsp) {
    match lsp {
        Lsp::Off => None,
        Lsp::Auto => { /* … constructs the language-server client here … */ }
    }
}
```

So the tool registry **constructs a language server**, and that is the only reason the crate depends
on `epsilon-lsp`. The architecture law says LSP communicates through contracts; this edge is the
violation, and it is not a layering nicety — it means every consumer of `epsilon-tools` (including
tests and any future frontend) links a language-server client whether or not it wants diagnostics.

## The inversion, which is the one already in the tree

The *policy* ("should a server be started for this workspace?") belongs to the frontend, which knows
the flags; the *handle* ("here is something that can answer for a file") is all the tool needs. That
is `TaskHost` and `GoalHost` again, and the plan is deliberately shaped after them:

1. **`epsilon-contracts` gains a seam trait**, minimal and derived from the call sites rather than
   invented: the write and edit tools currently hold "optional LSP feedback on what was just
   written/edited" (`epsilon-tools/src/files.rs:261`, `:422`). Read those two sites first and extract
   *only* the methods they call — a trait that guesses wider than its callers is how a seam becomes an
   abstraction.
2. **`epsilon-tools` takes the handle**: `register_builtins_with(registry, confinement, diagnostics:
   Option<Arc<dyn DiagnosticsHost>>)`. `Lsp::Off`/`Lsp::Auto` moves to the caller that decides it —
   the CLI, which already passes an `lsp:` value into `RunParams`.
3. **Drop the dependency** from `epsilon-tools/Cargo.toml`, then prove it:
   `cargo tree -p epsilon-tools -i epsilon-lsp` must report that nothing depends on it from there.
   (A manifest edit alone is not proof — the same trap plan 019 step 2 names for `toml`.)
4. **`cargo check -p epsilon-cli --features real`** as well as the workspace, because the CLI is the
   caller that now owns the construction and the `real` feature changes the path.

## Evidence

- the existing LSP-feedback tests keep passing **with the handle injected**, which is the evidence that
  the inversion is behaviour-preserving; if they need a live server, they were testing the
  construction rather than the feedback and should be split;
- `cargo tree -i epsilon-lsp` shows the frontend as the only dependent;
- battery `pwsh -File scripts/gate.ps1 m215` with the M208 disk knob, and `check-lint-tables` watched
  (it compares every crate's `[lints]` table with the workspace's, and a manifest edit is exactly what
  it reads).

## What this deliberately does not do

- **No new crate.** ADR-014's rule; this removes an edge rather than adding a brick.
- **No behaviour in `epsilon-contracts`.** A trait with no default body and no state is the M89
  exception's shape, not an extension of it — but if a *default method* turns out to be convenient,
  that is the line the architecture section draws, and it does not get crossed for tidiness.
- **No change to when diagnostics run.** `Auto` still means "start one for this workspace"; only the
  place that decides and the place that constructs change.

## Limits of this plan

- **The trait's shape is not written down here, on purpose.** It depends on what `files.rs` actually
  calls, and inventing it from memory is the failure mode this repository has paid for four times
  (A7, A10, A11, A20 were all rows written from a neighbouring fact). Step 1 is a read, not a design.
- **The estimate assumes the diagnostics path is injectable in one step.** If the write/edit tools
  cache a client *per tool instance* rather than per call, the handle has to reach further than
  `register_builtins_with` and the slice grows; that is the first thing to check.
