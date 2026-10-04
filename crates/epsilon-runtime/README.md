# epsilon-runtime

## What this crate is

Composition: the one place a workspace plus a configuration becomes a running agent.
`resolve_launch` decides which provider and model a launch gets; `build_runtime` turns
those values into tools, permission layers, skills, extensions, an MCP attachment, the
session store and the `Agent`. Every front end calls these two. It never draws and never
parses a command line; what a front end does with the agent is its own business.

## How it connects

- **Depends on:** all nine library crates — `epsilon-contracts`, `epsilon-llm`,
  `epsilon-core`, `epsilon-tools`, `epsilon-lsp`, `epsilon-safety`, `epsilon-session`,
  `epsilon-ext`, `epsilon-mcp`.
- **Depended on by:** the three front ends, `epsilon-cli`, `epsilon-tui`, `epsilon-desktop`.

## Entry points

- `resolve_launch` (`src/lib.rs:5230`) — `LaunchRequest` (`:5135`) in, `Launch` (`:5186`)
  out: model-reference resolution, credential-collision substitution, the real-vs-faux
  decision, project trust, ordered startup notices. Errors are `String`: the caller's
  sentence to show.
- `build_runtime` (`src/lib.rs:4448`) — `RunParams` (`:4376`) in, `Arc<Agent>` out.
  `build_runtime_and_gate_runner` (`:4460`) is the same call plus the goal-gate runner,
  which shares the agent's sandbox.
- Trust: `load_project_config` (`:154`) parses `.epsilon/config.toml` and hashes it;
  `ProjectTrustStore` (`:387`) is the remembered per-path decision;
  `trust_requiring_resources` (`:627`) decides what an untrusted workspace may not load.
  Roots: `skill_roots_trusted` (`:713`), `mcp_servers_trusted` (`:755`).
- `attach_extensions` (`:888`) spawns extensions and namespaces their tools
  `ext__{extension}__{tool}`; `install_extension_hooks` (`:1324`) and
  `install_extension_turn_hooks` (`:1308`) wire ADR-019 interception onto the agent.
  `scrub_env` (`:877`) is the credential scrubber handed to every child process.
- `text_safety::display_safe` (`src/text_safety.rs:92`) strips bidi and control characters
  from text a human will be asked to approve; `ask_is_display_honest` (`src/lib.rs:3931`)
  says whether such an ask may be approved at all.

## How to run and test it

No binary of its own — `epsilon-cli`, `epsilon-tui` and `epsilon-desktop` are what call it.
Integration tests: `tests/goal_round_gates.rs`, `tests/goal_gated_completion.rs`; an
offline run through a composed agent is `cargo run -q -p epsilon-cli -- --demo`.

```sh
cargo check -p epsilon-runtime
cargo test -p epsilon-runtime
```

## Things worth knowing

- One composition point. A second copy of this inside a front end is a defect, not an
  optimisation — that is the whole reason the crate exists (`src/lib.rs:4`).
- The order inside `build_runtime` is load-bearing. The resume target is resolved *before*
  anything is opened, and the store is attached before the model view is restored
  (`src/lib.rs:4431`). Resolving `--continue` afterwards made it pick the file this
  process had just created, wait out its own lease and restore nothing.
- `default = ["real"]` forwards to `epsilon-core/real` (`Cargo.toml:10`). Without it
  `Agent::try_hot_swap_to_real` compiles to a stub returning `false` — silently.
- Trust is two mechanisms: the config hash answers "did this file change", the trust store
  answers "did the user trust this directory" (`src/lib.rs:86`). A remembered `false`
  denies even an unchanged config, and an untrusted project loses its skills, extensions,
  agent definitions **and** MCP servers — the last because `[[mcp_servers]]` entries are
  commands to spawn, outside the sandbox and before any permission ask (`:731`).
  `ExtensionToolHooks` treats any extension veto as a veto (`:949`).