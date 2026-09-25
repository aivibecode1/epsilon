# Install

For developers trying a reviewed source build. Prerequisite: **Rust 1.88 or newer**
([rustup.rs](https://rustup.rs)) and its platform build prerequisites. Windows and
Linux are development targets, not a certified release matrix. Windows verification
used Rust 1.97.1; macOS execution was not tested and has no native shell-confinement
backend. Default shell execution refuses when no backend exists. Linux needs
`bwrap`; shell/LSP/MCP/extension programs must be installed when used. See
[permissions](permissions.md).

This is a development build, not a release: it is not certified on Linux or macOS beyond
CI compiling it, and the defects still open are tracked in
[the defect ledger](../../audit/02-defect-ledger.md). What the recorded verification does and
does not prove is in [the current assessment](../../audit/11-current-assessment.md); read that
before trusting a claim here. Run the commands below from the repository root.

```sh
cargo install --path crates/epsilon-cli --force     # installs `epsilon` into ~/.cargo/bin
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
epsilon -p "summarize this repo" # headless: reply to stdout, diagnostics to stderr
```

In the TUI: type a task, press Enter; `Esc` stops a run, `/help` lists slash commands,
`epsilon --help` has the full reference.
