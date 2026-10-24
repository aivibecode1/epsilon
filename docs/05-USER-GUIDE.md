# Using Epsilon — task map

Product guides. The installed binary is its own reference: `epsilon --help` prints every
mode, flag, environment variable and file path, and is kept in one place in the source
(`crates/epsilon-cli/src/main.rs`, the `HELP` constant). **Its scope is modes, flags,
environment and file paths — not slash commands.** Commands are defined once in
`crates/epsilon-contracts/src/command.rs`, drawn by the `/` completion and the command
palette, and documented in [`07-COMMANDS.md`](07-COMMANDS.md). When a guide and the
binary disagree, inspect the implementation; the help text is not a specification.

| Guide | You want to… |
|---|---|
| [`install.md`](user/install.md) | install Epsilon, give it a model, verify the loop works offline |
| [`configuration.md`](user/configuration.md) | configure models, custom providers, client ids, editors, keybindings |
| [`auth.md`](user/auth.md) | manage API keys and OAuth subscriptions, fix credential problems |
| [`permissions.md`](user/permissions.md) | understand permission rules, presets, sandbox modes, `--plan`, headless behavior |
| [`models.md`](user/models.md) | pick a model, read live listings and their age, understand retired badges |
| [sessions.md](user/sessions.md) | resume, fork, export, recover damaged transcripts, leases |
| [`goals.md`](user/goals.md) | park one long-running objective, watch it, stop it, understand what it costs |
| [`skills.md`](user/skills.md) | see the built-in skills, enable one, write your own |
| [`subagents.md`](user/subagents.md) | choose a built-in subagent role for delegated work, or write one |
| [`reverse-engineering.md`](user/reverse-engineering.md) | attach REA and inspect a binary, app or site with evidence |
| [`extensions.md`](user/extensions.md) | write or install a helper process that sees or answers tool calls |
| [`prompt-templates.md`](user/prompt-templates.md) | reuse a saved prompt, and find out which ones you have |
| [`troubleshooting.md`](user/troubleshooting.md) | diagnose a run that fails, hangs, or refuses |
| [`09-TUI-REFERENCE.md`](09-TUI-REFERENCE.md) | learn the frame, focus rules and every default key |
| [`10-EXTENSION-POINTS.md`](10-EXTENSION-POINTS.md) | choose the right seam to extend Epsilon |

What lives where:

- Product-use material belongs in **this folder**, one owner per fact.
- Current limitations and security decisions are in
  [`04-STATUS.md`](04-STATUS.md). Do not turn an open limitation into a usage
  instruction; state it where the guide needs it.
- Slash-command spellings and argument shapes live in
  [`07-COMMANDS.md`](07-COMMANDS.md).
