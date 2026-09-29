# Using Epsilon — task map

Product guides. The installed binary is its own reference: `epsilon --help` prints every
mode, flag, environment variable and file path, and is kept in one place in the source
(`crates/epsilon-cli/src/main.rs`, the `HELP` constant). When these guides and the binary
disagree, inspect the implementation and the known audit findings — the help text is not a
specification. The two drifts this paragraph used to warn about are fixed: the `--plan` line
now says `bash`/`shell` are denied outright, and `RUST_LOG` is no longer advertised, because
nothing in the workspace reads it.

| Guide | You want to… |
|---|---|
| [`install.md`](install.md) | install Epsilon, give it a model, verify the loop works offline |
| [`configuration.md`](configuration.md) | configure models, custom providers, client ids, editors, keybindings |
| [`auth.md`](auth.md) | manage API keys and OAuth subscriptions, fix credential problems |
| [`permissions.md`](permissions.md) | understand permission rules, sandbox modes, `--plan`, headless behavior |
| [sessions.md](sessions.md) | resume, fork, export, recover damaged transcripts, leases |
| [`goals.md`](goals.md) | park one long-running objective, watch it, stop it, understand what it costs |
| [`troubleshooting.md`](troubleshooting.md) | diagnose a run that fails, hangs, or refuses |

What lives where:

- Product-use material belongs in **this folder**, one owner per fact.
- Historical capability claims (what was true at milestone N) stay in
  [`../milestones/`](../milestones) and [`../history.md`](../history.md) — they are records,
  not current references.
- Known current defects and their evidence live in [`../../audit/03-defects.md`](../../audit/03-defects.md);
  do not turn a finding into a usage instruction while it is open.
