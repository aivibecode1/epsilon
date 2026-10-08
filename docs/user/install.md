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
[the defect ledger](../../audit/03-defects.md). What the recorded verification does and
does not prove is in [the 2026-09-16 assessment (history)](../../audit/history/11-current-assessment.md); read that
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

## Preview scope (honest version)

This is a preview, not a release: no versioned artifacts, install is from
source, and the gap lists are the parity matrices —
[Pi port](../parity/pi-tui.md) (50 done of 339) and
[DSH Desktop](../parity/dsh-desktop.md) (17 done of 433).

What works: the agent loop with tools, approvals and checkpoints; 38 slash
heads in the terminal and 37 in the native desktop window (which answers
everything the terminal does except `/editor`); sessions shared between both
front ends; permission presets with a risk gate on the way up. What does not:
much of both matrices, a screen-reader pass (never done), context menus, and
any platform beyond the dev machines. The defect ledger is
[audit/03-defects.md](../../audit/03-defects.md). The install this was
verified with — commands, outputs, hashes — is
[the preview transcript](../../audit/evidence/preview-install-2026-10-08.txt).
