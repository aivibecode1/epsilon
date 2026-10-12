# epsilon-safety

## What this crate is

Layer 1 of the two-layer safety model: permission rules and the approval seam. A flat list
of `{capability, pattern, action}` rules is evaluated across stacked layers, and an `ask`
is handed to a front-end-supplied `ApprovalUi`. Layer 2 — OS confinement for child
processes — is here too. It does not own the tools, and it never draws the dialog: the
front end implements `ApprovalUi` and owns the interaction.

## How it connects

- **Depends on:** `epsilon-contracts`, `epsilon-core`, `winapi` on Windows. `epsilon-llm`
  and `epsilon-tools` are dev-dependencies, so its tests can drive a real loop against a
  real registry.
- **Depended on by:** `epsilon-tools`, and through it `epsilon-runtime`, `epsilon-tui`,
  `epsilon-server`, `epsilon-cli`.

## Entry points

- `engine` — `PermissionEngine` (`src/engine.rs:63`), `Rule` (`:23`), `Ruleset` (`:50`),
  `Action` (`:10`), `default_ruleset` (`:105`), `plan_ruleset` (`:137`).
- `gate` — `ApprovalUi` (`src/gate.rs:14`), the trait a front end implements;
  `RulesetGate` (`:51`) bridges the engine onto the loop's `PermissionGate` seam;
  `AutoApprove` (`:20`) and `AutoDeny` (`:32`) are the unattended stand-ins.
- `channel_ui` — `ChannelApprovalUi` (`src/channel_ui.rs:43`) parks asks by request id so
  a dialog loop or an HTTP handler can answer them out of order; `PendingAsk` (`:17`).
- `sandbox` — the `Sandbox` trait (`src/sandbox.rs:197`), `best_available` (`:321`),
  `ConfinementPolicy` (`:27`), `Enforcement` (`:86`), `NullSandbox` (`:234`),
  `BwrapSandbox` on unix (`:390`), `JobObjectSandbox` on Windows (`jobobject.rs:208`).
- `escalation` — the `SandboxMode` ladder (`src/escalation.rs:26`) and the model-facing
  refusal text (`denial_marker`, `:216`). `env` — `scrub_secret_env` (`src/env.rs:79`).

## How to run and test it

No binary of its own. To see an approval in action, run the terminal front end and call
something the default rules ask about. Integration tests: `tests/loop_with_rules.rs`,
`tests/plan_shell.rs`, `tests/additive_authorization.rs`, `tests/bwrap_unix.rs` (unix).

```sh
cargo check -p epsilon-safety
cargo test -p epsilon-safety
```

## Things worth knowing

- Evaluation is last-match-wins across layers, except that any deny returns immediately
  (`src/engine.rs:83`). A project layer cannot re-allow what a global layer denied, and a
  capability with no matching rule resolves to `Ask`, never `Allow` (`:97`).
- Deny is monotonic. `RulesetGate::authorize` evaluates for a deny *before* consulting
  session memory, so an "always allow" can only ever convert an `Ask` (`src/gate.rs:85`).
- The approval channel fails closed: a consumer that drops a `PendingAsk` without replying
  resolves the blocked call to `Deny` (`src/channel_ui.rs:128`).
- A sandbox reports what it could **not** enforce in `Enforcement::missing`, and a caller
  that needs confinement must treat `available() == false` as refuse-to-run
  (`src/sandbox.rs:320`). `scrub_secret_env` drops any credential-shaped environment
  variable before a child starts (`src/env.rs:79`).
- `sandbox/jobobject.rs` is the only `unsafe` in the workspace. Because `unsafe_code =
  "forbid"` cannot be narrowed from below, this crate's `Cargo.toml` carries a hand-written
  copy of the workspace lint table with `unsafe_code = "allow"` and
  `undocumented_unsafe_blocks`; `scripts/check-lint-tables.py` fails if the two drift.