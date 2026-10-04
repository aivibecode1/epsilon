# epsilon-tui

The terminal frontend. It draws with ratatui 0.30.2 and reads keys with crossterm 0.29.

## What this crate is

It owns pixels and key handling for the terminal window: layout, transcript, composer,
overlays, and the reducer that turns a key press or an agent event into the next frame. It
does **not** own agent behaviour — the loop, the tools, the permission gates and compaction
belong to `epsilon-core`.

One thing to know before reading: this crate still *hosts* the shared theme roles
(`ui::theme`) and `epsilon-desktop` imports them. That is where the code is today; it is not a
second copy of anything. The **slash-command half moved out**: `parse_command`, `Command`,
`SLASH_ITEMS`, `COMMAND_HEADS` and `ALIAS_HEADS` now live in
[`epsilon-contracts`](../epsilon-contracts/README.md) (`src/command.rs`) and are re-exported from
`picker`, so every path below still resolves. The picker half stays here because it needs
`epsilon_llm::catalog`, and contracts has no internal dependencies.

## How it connects

Workspace dependencies: `epsilon-contracts`, `epsilon-core`, `epsilon-llm`,
`epsilon-safety`, `epsilon-session`, `epsilon-runtime`. External: ratatui, crossterm,
arboard, base64, pulldown-cmark, unicode-width, unicode-segmentation, futures-util, tokio,
serde_json. Depended on by `epsilon-cli` and `epsilon-desktop`, and by nothing else.

## Entry points

| Item | Where |
|---|---|
| `app::run` — run the event loop until the user quits | `src/app.rs:1342` |
| `app::TuiConfig` — the agent, its approvals, an optional live model fetcher | `src/app.rs:37` |
| `state::UiState` — the pure event reducer | `src/state.rs:539` |
| `picker::parse_command` — the slash-command parser (re-exported) | `crates/epsilon-contracts/src/command.rs:464` |
| `ui::theme::Theme` / `ThemeMode` — shared colour roles | `src/ui/theme.rs:858`, `src/ui/theme.rs:110` |
| `ui::metrics::Metrics` — context, cache and cost figures | `src/ui/metrics.rs:35` |
| `keybindings::KeybindingsManager` | `src/keybindings.rs:286` |

## How to run and test it

This crate ships no binary. The CLI starts it:

```sh
cargo run -q -p epsilon-cli            # interactive TUI
cargo test -p epsilon-tui
```

Its tests live inside `src/` — `#[cfg(test)]` modules plus `src/app/tests.rs` and
`src/app/golden.rs`. There is no `tests/` directory.

## Things worth knowing

- `real` is a **default** feature (`Cargo.toml:23`). Without it
  `Agent::try_hot_swap_to_real` compiles to a `false` stub, and the "now live on X" hot-swap
  silently degrades to "restart epsilon". Air-gapped builds use `--no-default-features`.
- A panic must leave a usable console, so the terminal restore lives in a `Drop` rather than
  an error path (`src/app.rs:1254`). Permission asks travel through
  `epsilon_safety::ChannelApprovalUi`, the channel the HTTP server also answers from.
- Changing `ui::theme` changes the desktop too: see `crates/epsilon-desktop/src/theme.rs:25`.
  Changing `parse_command` does **not** go through this crate any more — the desktop reads
  `epsilon_contracts::command` directly.
- Rules: [`AGENTS.md`](../../AGENTS.md),
  [`docs/design/epsilon-decisions.md`](../../docs/design/epsilon-decisions.md).