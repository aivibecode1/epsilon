# Extension points

Epsilon is one core with several frontends. This page is the map of the seams
where another program, another model, or another tool can join without forking
the core. Each seam names the contract, the file that owns it, and the limit.

Use this page to choose a seam. Use the linked guide for the full protocol.

## 1. Tools

A tool is anything that implements `epsilon_contracts::tool::Tool`: a
`ToolDescriptor`, an async `execute`, and a declared capability.

| Question | Answer |
|---|---|
| Where is the trait? | `crates/epsilon-contracts/src/tool.rs` |
| Where are built-ins registered? | `crates/epsilon-tools/src/lib.rs` |
| How is permission decided? | `Tool::capability()` → `epsilon-safety` layers |
| What must a new tool do? | Declare honest capability, bound its own input/output, respect `ctx.token` cancellation |

Adding a tool to the core is a code change with tests. Adding one from outside
the binary uses one of the next two seams.

## 2. MCP servers

Model Context Protocol servers are the supported way to add tools without
changing Epsilon. A server is a child process over stdio or a StreamableHTTP
endpoint; its tools register as `mcp__<server>__<tool>`.

```toml
[[mcp_servers]]
name = "my-server"
command = "my-server"
args = ["--stdio"]
timeout_secs = 300          # optional; default 120
```

Or for one run: `epsilon --mcp my-server=my-server:--stdio`. HTTP endpoints use
`url` and may carry headers/bearer tokens from the config file only, never a
command line. Project-declared servers start only after the workspace is
trusted. A server that fails to start refuses the launch rather than silently
running without its tools.

Full shape: [`user/extensions.md`](user/extensions.md) and
`crates/epsilon-runtime/src/lib.rs` (`[[mcp_servers]]` parser).

## 3. Process extensions

An extension is a separate program speaking one JSON object per line over stdin
and stdout. It can interpose on events (notably `before_tool_call`), block a call
with a reason, and provide tools. It cannot reach into the agent's memory or
widen its own permissions; the worst it can do is stop working.

`extension.json` declares `name`, `version`, `bin`, `args`, `events` and `tools`.
Extensions are discovered per user/project root and gated by project trust.

Full protocol: [`user/extensions.md`](user/extensions.md). Contract and host:
`crates/epsilon-ext`.

## 4. Skills

A skill is a `SKILL.md` file with frontmatter (`name`, `description`, optional
`disable-model-invocation`) and a body. Discovery reads the standard roots used
by Claude Code, Codex, Gemini CLI, Cursor, Windsurf, opencode, Pi, DeepSeek
Harness and Epsilon itself. The catalog gets the names and descriptions; the body
loads only when the model calls the `skill` tool or a user invokes it.

This is the cheapest extension point: no code, no process, no schema. It is also
the one that spends context, so the catalogue is capped and skills are opt-in
when bundled. See [`user/skills.md`](user/skills.md).

## 5. Prompt templates and custom commands

A Markdown file in `.agents/prompts/` or `.epsilon/prompts/` becomes a
`/name` prompt template. A file under `commands/` becomes a command. A template
cannot shadow a builtin. Argument hints come from frontmatter.

See [`user/prompt-templates.md`](user/prompt-templates.md).

## 6. Agent definitions

An agent definition is a Markdown file with frontmatter that names a role, a
model and a tool subset. The `task` tool routes delegated work to a matching
definition; a child inherits the parent's skills and overlays but gets its own
session and depth. Definitions are trusted-tier resources.

Four **built-in subagent roles** (`explore`, `review`, `verify`, `implement`)
ship in the binary. They are opt-in: `/subagent <name> on` writes an ordinary
definition under `<config dir>/builtin-agents/`, and `/subagent` opens the
browser with what each does and why. See [`user/subagents.md`](user/subagents.md).
The naming follows DeepSeek Harness: the role is a subagent, while "harness"
names the whole program.

See `crates/epsilon-core/src/agents.rs` and the `task` tool description.

## 7. Frontends

The HTTP/SSE server and stdio-RPC mode are frontends over the same agent.
`epsilon-runtime::build_runtime` is the only composition point; a new frontend
calls it rather than rebuilding provider/skill/permission setup.

| Mode | Entry | Protocol |
|---|---|---|
| Terminal IDE | `epsilon` | full-screen TUI |
| Headless | `epsilon -p "…"` | text to stdout, diagnostics to stderr |
| HTTP/SSE | `epsilon --serve` | `POST /prompt`, SSE events |
| stdio RPC | `epsilon --rpc` | NDJSON request/response |
| Demo | `epsilon --demo` | deterministic faux provider, no network |

The RPC command list is advertised but slash commands are not executed there;
the text is sent to the model verbatim.

## 8. Models and providers

Providers are declared in `epsilon-llm`; custom OpenAI-compatible providers are a
config table, not code. Model routing, retry and fallback are in the same crate.
A new provider protocol is a code contribution with tests against a faux server;
a new provider *endpoint* is configuration.

See [`user/models.md`](user/models.md) and [`user/configuration.md`](user/configuration.md).

## 9. Themes and keybindings

`[keys]` in `config.toml` rebinds the declared ids; `/hotkeys` prints what is
effective. Themes are tokens, not scattered colours. A theme change is a
`ui/theme.rs` change with contrast assertions, not a new draw path.

## The rule that applies to every seam

**One composition point, one decision.** If a new extension point needs to know
something the core already knows (which skills are enabled, which permission
layers apply, which model is routed), it asks the existing owner. A second copy
of a rule is the defect this repository's architecture is built to prevent.
