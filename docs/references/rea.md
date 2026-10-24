# REA (Reverse Engineer Anything)

**What it is.** REA is an MCP server and CLI for inspecting binaries, desktop and
Electron applications, .NET assemblies, websites, Android packages and captured
traffic. It returns observations with the evidence and the limitations behind
each one. It is a separate project, not a harness.

| Fact | Value |
|---|---|
| Name | REA: Reverse Engineer Anything |
| Author | morluto |
| Repository | <https://github.com/morluto/rea> |
| Package | `rea-agents` (npm) |
| Version studied and pinned | `6.7.0` |
| Licence | MIT — "Copyright (c) 2026 morluto" |
| Vendored | **No.** Epsilon invokes the published package over MCP. |

## Why Epsilon integrates it

Epsilon is a coding harness. Its tools read, edit, search and run code; they do
not understand a stripped binary or a shipped Electron bundle. REA adds that
capability without adding a second plugin system: it is one MCP server, and
Epsilon already has a tested MCP client.

The integration is deliberately thin:

- `--rea` attaches the server for one run.
- `[[mcp_servers]]` in the workspace `.epsilon/config.toml` is the permanent
  route, and works for any MCP server.
- `timeout_secs` exists because REA's Ghidra/Hopper analysis legitimately
  outlasts the 120-second default.
- The Windows wrapper (`cmd.exe /d /c npx …`) is encoded in
  `epsilon-runtime::integrations`, not left to each user to discover.
- The built-in `reverse-engineer-anything` skill carries the workflow and the
  evidence rules. It is opt-in like every built-in.

There is no code copied from REA, no vendored schema, and no REA tool registered
when the integration is off. The version is pinned; updating it is a deliberate
change with a fresh check of the package's MCP contract.

## What it taught, and what Epsilon does not copy

**Adopted:** evidence-first analysis. Every REA result carries the provider and
its limits; Epsilon's own reporting rule is the same one (see the built-in
`verify-before-claim` skill).

**Not copied:** REA's provider lifecycle, its large tool catalogue, and its
package-runner install story. Epsilon wants one optional switch and a pinned
version, not a second runtime to keep current.

**The honest limit:** REA's native analysis depends on external engines. When a
provider is absent, the correct answer is "unavailable, here is why", not a
guess. REA already reports that; the skill tells the model to repeat it.

## Using it

See [Reverse engineering with REA](../user/reverse-engineering.md). The one-line
version:

```sh
epsilon --rea
```

Then start with `binary_session` and an empty argument; it reports what this
machine can actually do.
