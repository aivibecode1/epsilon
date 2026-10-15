# Configuration

For users selecting models and project settings.

Global configuration is `~/.config/epsilon/config.toml`; project configuration is
`<workspace>/.epsilon/config.toml` and is **trust-gated**: the first time the file's content
hash changes, Epsilon asks you to trust it (`/trust` in the TUI) before applying it.
Precedence (for model id, thinking level, and permission preset — the keys both
files carry): command-line flag → project config → global config → catalog default (ADR-018).
Other keys (theme, favorites, editor, skills-disabled, custom providers, OAuth client
ids, keybindings) are global-only and never read from the project file.

## Model selection

```sh
epsilon --model <id>                 # catalog id, alias, or provider/model
epsilon --fallback <id>              # repeatable; tried in order on retryable failures
```

`id` is resolved against the bundled catalog (`epsilon --list-models`).

## Custom providers

```toml
[provider_custom.lmstudio]
base_url = "http://localhost:1234/v1"
# env_key = "LMSTUDIO_API_KEY"   # optional for local servers
```

```sh
epsilon --model lmstudio/qwen2.5-7b-instruct -p "hi"
```

## OAuth client ids

```toml
[oauth.anthropic]
client_id = "…"
```

See [`auth.md`](auth.md) for client-id resolution and refresh limitations.

## Editor

```toml
[editor]
command = "code --wait"   # used by Ctrl+O / Alt+E and /editor
```

## Launch modes

Two flags change what the agent may do before it answers anything. Both are
composed into the permission engine's layer stack at startup, so they hold for the
whole run regardless of what the model asks for.

| Flag | Effect |
|---|---|
| `--plan` | Plan mode. A layer that denies every mutating capability is prepended to the engine and monotonic-deny keeps that boundary against project rules and mid-run approvals (ADR-006's F10 amendment). Reads, searches and diagnostics still work; writes, edits and shell commands that change anything are refused with the reason. There is no exit-normal transition — to allow edits, restart without the flag |
| `--preset <name>` | Set the permission baseline: `auto` (default), `workspace-write`, `read-only`, `danger-full-access`. Only `danger-full-access` is ephemeral (re-confirmed each launch, never written down); the other three persist through the TUI to the project config first, then the global one |
| `--fallback <model>[,<model>…]` | After this flag's model list, fall through to the next candidate when a call fails for a retryable reason. Each candidate resolves its own credentials at stream time |
| `-c` / `--continue` | Resume the newest session for the current workspace instead of starting a new one |

`--plan` and `--preset` stack: plan mode's deny layer is prepended first, so a
`danger-full-access` preset cannot reopen mutations under `--plan`.

## Environment

| Variable | Purpose |
|---|---|
| `<PROVIDER>_API_KEY` | credential fallback when nothing is stored |
| `RUST_LOG` | **does nothing.** No crate in this workspace depends on `tracing`/`tracing-subscriber` and none installs a subscriber, so nothing reads the variable; `epsilon --help` no longer lists it, and headless diagnostics go to stderr regardless |
| `EPSILON_STREAM_IDLE_SECS` | seconds of stream silence before a turn fails (default 120; `0` disables) |

## Files on disk

| Path | What it is |
|---|---|
| `~/.config/epsilon/config.toml` | global config |
| `~/.config/epsilon/auth.json` | stored credentials |
| `~/.config/epsilon/MEMORY.md` | global memory injected into the prompt |
| `~/.config/epsilon/history` | prompt history |
| `~/.config/epsilon/keybindings.json` | remapped keys |
| `~/.config/epsilon/skills/` | global skills (`SKILL.md` folders) |
| `~/.claude/skills/`, `~/.codex/skills/`, `~/.gemini/skills/`, `~/.agents/skills/` | global skills written for another harness — read here too |
| `<workspace>/.epsilon/config.toml` | project config (trust-gated) |
| `<workspace>/.epsilon/sessions/<workspace>/*.jsonl` | session transcripts, one file per launch (older sessions may sit directly in `.epsilon/sessions/`) |
| `<workspace>/.epsilon/skills/`, `<workspace>/.agents/skills/`, `<workspace>/.claude/skills/`, `<workspace>/.codex/skills/`, `<workspace>/.gemini/skills/`, `<workspace>/.cursor/skills/`, `<workspace>/.windsurf/skills/`, `<workspace>/.opencode/skills/`, `<workspace>/.dsh/skills/` | project skills (trust-gated) |
| `<workspace>/AGENTS.md`, `CLAUDE.md`, `MEMORY.md` | project instructions and memory, injected into the prompt's CONTEXT tier |
