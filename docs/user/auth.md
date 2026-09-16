# Auth

For users configuring their own provider accounts.

Credentials live in `~/.config/epsilon/auth.json` (written with restrictive permissions
where the OS supports it). Precedence: **stored credentials win over environment variables.**

## API keys

```sh
epsilon auth list                     # stored providers, keys masked
epsilon auth login <provider>         # enter key through the local prompt
epsilon auth logout <provider>
```

Environment variables are the fallback, useful in CI: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
`DEEPSEEK_API_KEY`, `GEMINI_API_KEY`, `XAI_API_KEY`, `OPENROUTER_API_KEY`, `MOONSHOT_API_KEY`,
`ZAI_API_KEY`, `GROQ_API_KEY` (see `epsilon --help` → ENVIRONMENT).

## OAuth (subscription) logins

```sh
epsilon auth login anthropic --oauth        # Claude Pro/Max
epsilon auth login openrouter --oauth
epsilon auth login xai --oauth
epsilon auth login github-copilot --oauth   # GitHub Copilot subscription
```

- `--paste` skips the loopback callback listener and asks you to paste a redirect URL — for
  when the browser runs on a different machine (SSH, remote dev).
- Flows requiring an OAuth application need your registered **client id**; do not
  assume Epsilon embeds another project's registration. Resolution uses the flow's
  environment variable, then configuration, otherwise returns an error. Configure
  the appropriate flow in `~/.config/epsilon/config.toml`:

  ```toml
  [oauth.anthropic]
  client_id = "…"
  ```

  Details and provider-by-provider notes:
  [`../qa/08-oauth-login-client-ids.md`](../qa/08-oauth-login-client-ids.md).

- Refresh is implemented but not reliable across all paths: configuration-only IDs,
  Copilot preparation, proactive rotation and cached fallback credentials have open
  integration findings (F16). A successful login is not proof of later refresh.
  See [the ledger](../../audit/02-defect-ledger.md).

## Custom providers with credentials

Local servers (LM Studio, vLLM, anything OpenAI-compatible) need no key. Cloud providers not
in the catalog can be added with an `env_key`; see [`configuration.md`](configuration.md).
