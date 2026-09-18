# Configuration

For users selecting models and project settings.

Global configuration is `~/.config/epsilon/config.toml`; project configuration is
`<workspace>/.epsilon/config.toml` and is **trust-gated**: the first time the file's content
hash changes, Epsilon asks you to trust it (`/trust` in the TUI) before applying it.
Precedence: command-line flag → project config → global config → catalog default (ADR-018).

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

## Environment

| Variable | Purpose |
|---|---|
| `<PROVIDER>_API_KEY` | credential fallback when nothing is stored |
| `RUST_LOG` | advertised by help, but no application tracing initialization was established in the audit; do not rely on it for diagnostics |
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
| `<workspace>/.epsilon/config.toml` | project config (trust-gated) |
| `<workspace>/.epsilon/sessions/*.jsonl` | session transcripts |
| `<workspace>/.epsilon/skills/`, `<workspace>/.agents/skills/` | project skills |
| `<workspace>/AGENTS.md`, `CLAUDE.md`, `MEMORY.md` | project instructions and memory, injected into the prompt's CONTEXT tier |
