# epsilon-desktop

The native desktop IDE. egui/eframe 0.36.2, rendered natively through winit and wgpu.

## What this crate is

A real desktop application in an operating-system window — not a webview, not an embedded
browser, not an Electron shell. That is a recorded invariant (ADR-020), not a preference.

It owns pixels and nothing else. Every turn, tool call, permission decision and compaction
here is the same `epsilon_core::Agent` the terminal drives, over the same session files,
behind the same approval seam. It composes that agent through `epsilon_runtime` (ADR-021),
and does **not** own the agent loop, the permission engine, or composition.

## How it connects

Workspace dependencies: `epsilon-contracts`, `epsilon-core`, `epsilon-llm`,
`epsilon-runtime`, `epsilon-session`, `epsilon-safety`, `epsilon-tools`, `epsilon-mcp`,
`epsilon-tui`. External: ratatui, eframe, egui, pulldown-cmark, serde_json, tokio. Dev:
tempfile. Depended on by nothing here; `epsilon-tools` and `epsilon-mcp` are present for two
types that appear as `RunParams` fields, not for behaviour.

## Entry points

| Item | Where |
|---|---|
| `run(Options) -> Result<(), String>` — open the window and run | `src/lib.rs:118` |
| `IdeApp::new` — application state and the frame | `src/app.rs:1521` |
| `IdeSession::start` — compose the agent, then hand it to the window | `src/session.rs:822` |
| `IdeTheme` — the palette, converted from the terminal's roles | `src/theme.rs:141` |
| `IdeApp::try_command` — typed slash command, or a refusal by name | `src/commands.rs:111` |
| the binary's `main` | `src/main.rs:26` |

## How to run and test it

This crate needs **Rust 1.95**, because egui declares it; the rest of the workspace builds on
1.88. The floor is declared on this crate alone (`Cargo.toml:13`) on purpose.

```sh
cargo run -q -p epsilon-desktop              # opens on the current directory
cargo run -q -p epsilon-desktop path/to/repo --model id
cargo test -p epsilon-desktop
```

Flags are `--session FILE`, `--model ID`, `--light`, `--dark`.

## Things worth knowing

- The agent is composed **before** the window opens (`src/lib.rs:97`). A launch that fails
  produces a sentence and no window, rather than a window that cannot do anything.
- `run` returns `Err(String)`, not an `eframe::Error`: the binary adds the "could not open
  the Epsilon window" prefix (`src/main.rs:67`), and a shared function must never exit.
- The palette is borrowed from `epsilon_tui::ui::theme::Theme` and converted one way, ratatui
  colour to egui `Color32` (`theme.rs::to_color32`). A colour the window needs and the terminal
  lacks is a missing *role*, not a second palette.
- Tests render into an off-screen `egui::Context` — no window, no GPU — while driving the real
  draw path against a real composed agent (`tests/window.rs:1`).
- egui paints top to bottom, so a transcript row the composer adds during frame *N* appears in
  frame *N+1* (`tests/window.rs:23`). Assert "one frame later", not "immediately".
- Rules: [`AGENTS.md`](../../AGENTS.md),
  [`docs/design/epsilon-decisions.md`](../../docs/design/epsilon-decisions.md).