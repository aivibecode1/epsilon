# Install

For developers trying a reviewed source build. Prerequisite: **Rust 1.88 or newer**
([rustup.rs](https://rustup.rs)) and its platform build prerequisites. Windows and
Linux are development targets, not a certified release matrix. Windows verification
used Rust 1.97.1; macOS execution was not tested and has no native shell-confinement
backend. Default shell execution refuses when no backend exists. Linux needs
`bwrap`; shell/LSP/MCP/extension programs must be installed when used. See
[permissions](permissions.md).

This is a development build, not a release: it is not certified on Linux or macOS beyond
CI compiling it. The current limits and the security decisions behind them are in
[status](../04-STATUS.md). Run the commands below from the repository root.

## Use the installer

The installer builds the release binary with `cargo install --locked` and verifies
that it runs. It puts build output in a temporary target directory, not inside the
checkout, so it does not reproduce the in-tree compiler failures recorded in
[`../03-DEVELOPMENT.md`](../03-DEVELOPMENT.md).

**Windows** (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

**macOS and Linux:**

```sh
bash install.sh
```

Both accept a custom location and a no-verify mode:

```powershell
powershell -ExecutionPolicy Bypass -File .\install.ps1 -Prefix C:\Tools\epsilon
powershell -ExecutionPolicy Bypass -File .\install.ps1 -NoVerify
```

```sh
bash install.sh --prefix "$HOME/.local"
bash install.sh --no-verify
```

Without `--prefix`/`-Prefix`, the binary goes to Cargo's bin directory
(`~/.cargo/bin` or `$CARGO_HOME/bin`). The installer prints the exact path and, if
it is not on `PATH`, the line to add.

The equivalent manual command, if you want to run it yourself:

```sh
cargo install --path crates/epsilon-cli --locked --force
epsilon --help                                       # every mode, flag, env var and file
```

Real providers are compiled in by default; build with `--no-default-features` only if you
want an offline-only binary.

## Give it a model

```sh
epsilon --list-models                       # browse the bundled catalog, no key or network
epsilon auth login anthropic                # paste an API key once
epsilon auth login anthropic --oauth        # or log in with a Claude Pro/Max subscription
epsilon auth login github-copilot --oauth   # or a GitHub Copilot subscription
```

Stored credentials live in `~/.config/epsilon/auth.json` and win over environment variables
(`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `DEEPSEEK_API_KEY`, …), which are the CI fallback.
See [`auth.md`](auth.md).

## Verify it works offline

```sh
epsilon --demo      # scripted run against the deterministic faux provider: no key, no network
epsilon doctor      # checks model resolution, credentials, sandbox, sessions and trust
```

## First task

```sh
epsilon                          # interactive TUI in the current directory
epsilon -p "summarize this repo" # headless: reply to stdout; diagnostics, usage and cost to stderr
```

In the TUI: type a task, press Enter; `Esc` stops a run, `/help` lists slash commands,
`epsilon --help` has the full reference.

## Preview scope (honest version)

This is a preview, not a release: no versioned artifacts, install is from
source, and the gap list is the parity matrix —
[Pi port](../parity/pi-tui.md).

What works: the agent loop with tools, approvals and checkpoints; the slash heads in
the terminal; sessions with leases and repair; permission presets with a risk gate on the
way up. What does not: much of the matrix, a screen-reader pass (never done), context
menus, and any platform beyond the dev machines. The current limits are in
[status](../04-STATUS.md).
