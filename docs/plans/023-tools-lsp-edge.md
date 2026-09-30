# 023 — the `epsilon-tools` → `epsilon-lsp` edge, removed by inversion (A8)

**Ledger row:** A8. **Status: LANDED at M215** (battery `m215` PASS, 1,634 tests across 65 binaries;
[`docs/milestones/M215-tools-lsp-inversion.md`](../milestones/M215-tools-lsp-inversion.md)). The plan
below is kept as written — including the two claims its own step 0 corrected — because the way it was
wrong is the useful part. What actually landed: `Lsp` moved to `epsilon_cli::runtime` rather than
staying in `epsilon-tools` (a crate that no longer knows what a language server is should not export
a language-server policy), `diagnostics_for(policy)` is the one construction site, and
`register_builtins_with` takes `Option<Arc<dyn DiagnosticsProvider>>`. The dependency proof came out
stronger than step 3 asked for: `cargo tree -p epsilon-tools -i epsilon-lsp` does not report an empty
path, it reports that **no such package is in that crate's graph**.

**Precedent:** `ToolContext::task_host` and `ToolContext::goal_host` — the pattern this repository
already used twice for exactly this shape before this slice became the third.

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

## Step 0 — the seam is already there (checked 2026-09-30, and it changes the estimate)

**Plan 023's first step was written as "extract the trait's shape from the call sites". That work is
already done, and this paragraph exists because the plan was written from the ledger's description of
the row rather than from the code — the fifth time in one session that a claim inherited a neighbour's
shape. Checking cost one read:**

```rust
// epsilon-tools/src/files.rs
use epsilon_contracts::diagnostics::DiagnosticsProvider;   // :15 — the trait is in CONTRACTS

pub struct WriteTool {
    diagnostics: Option<Arc<dyn DiagnosticsProvider>>,      // :262 — the tool holds the seam
}
impl WriteTool {
    pub fn with_diagnostics(mut self, provider: Arc<dyn DiagnosticsProvider>) -> Self  // :285
}

async fn diagnostics_note(provider: Option<&Arc<dyn DiagnosticsProvider>>, path: &Path) -> String
```

So `epsilon-tools` **already talks to contracts, not to `epsilon-lsp`** for the feedback it needs: the
trait, the injection point and the "no provider means no block" rule are all in place, and the tool
never names the lsp crate. There is no trait to design.

**What that leaves is narrower and mechanical:** the only use of `epsilon-lsp` in the crate is the
`Lsp::Auto` arm that *constructs* the service and calls `with_diagnostics`. Move that construction to
the caller — the CLI, which already passes `lsp:` into `RunParams` — pass the resulting
`Arc<dyn DiagnosticsProvider>` (or `None`) into `register_builtins_with`, delete the enum and the
dependency. Steps 2–4 below stand unchanged; **the estimate drops from ~1 hour to well under it, and
the risk with it**, because nothing about the diagnostics behaviour is touched.

### And the caller is already able to do it (checked the same day)

The paragraph above says "move the construction to the CLI" as though that were a new capability. It
is not — the CLI already has everything it needs:

```text
crates/epsilon-cli/Cargo.toml   epsilon-lsp = { path = "../epsilon-lsp" }      <- the edge exists
doctor.rs:279                   lsp_servers: epsilon_lsp::DiagnosticsService::default()
main.rs:171                     lsp: epsilon_tools::Lsp                        <- the policy, passed in
main.rs:326/343/401-407         `--lsp auto|off` parsing, defaulting to Auto
```

So **no new dependency edge is created**, and the construction already exists in the CLI once (in
`doctor`) — which makes the change "build a `DiagnosticsService` where the enum is matched, and that
match now lives in `build_runtime` instead of the tool registry". Two consequences worth taking:

- **The `Lsp` enum does not have to move or die.** It is a policy name with no dependency of its own;
  what must leave `epsilon-tools` is the *match that builds a server*, not the enum. Keeping it means
  `RunParams.lsp: Lsp` is unchanged, `--lsp` parsing is unchanged, and the diff is one function's
  body — the smallest version of this slice, and the one to take.
- **`doctor` and `build_runtime` then construct the same thing in two places.** That is not a new
  duplication (doctor runs in its own mode and never shares a process with a turn), but it wants a
  named helper in the CLI — `fn diagnostics_for(policy: Lsp) -> Option<Arc<dyn DiagnosticsProvider>>`
  — so "off means None, auto means a default service" has one home. Otherwise this plan removes one
  rule from two places and creates another in two places, which is a poor trade.
## The inversion, which is the one already in the tree

The *policy* ("should a server be started for this workspace?") belongs to the frontend, which knows
the flags; the *handle* ("here is something that can answer for a file") is all the tool needs. That
is `TaskHost` and `GoalHost` again, and the plan is deliberately shaped after them:

1. ~~`epsilon-contracts` gains a seam trait.~~ **Superseded by step 0 — the trait exists and the tools
   already use it.** The work is moving the construction, not writing the seam.
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

- **The trait's shape *is* written down now** — step 0 read it — and the original refusal to write it
  from memory was right: the guess would have been a new trait in contracts, and the truth is that one
  already exists there. This plan was itself written from a neighbouring fact, which is the fifth
  instance this session (A7, A10, A11, A20, and now this). The countermeasure held: one read, no code.
- **The estimate assumes the diagnostics path is injectable in one step.** If the write/edit tools
  cache a client *per tool instance* rather than per call, the handle has to reach further than
  `register_builtins_with` and the slice grows; that is the first thing to check.
