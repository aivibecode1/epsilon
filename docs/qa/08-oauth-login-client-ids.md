# 08 — OAuth logins: client ids, provider rows, and feature forwarding

Conventions decided while landing M52 (OAuth login ceremonies). Three rules
worth writing down because they will come up again the next time someone adds a
provider or a feature.

## 1. Client ids resolve from the environment or the user's config — never embedded

`epsilon-llm/src/oauth_login.rs` resolves a flow's client id in this order:

1. `<PROVIDER>_OAUTH_CLIENT_ID` in the environment
   (`ANTHROPIC_OAUTH_CLIENT_ID`, `XAI_OAUTH_CLIENT_ID`, …),
2. `[oauth.<provider>] client_id` in `~/.config/epsilon/config.toml`,
3. otherwise a telling error naming both, plus the exact TOML snippet.

pi hardcodes one client id per provider in its own source (Anthropic's is
base64-obfuscated to dodge secret scanners). Copying another project's
registration into this tree is a supply-chain violation, not a convenience —
the same reasoning M46/M51 applied to refresh. The env/config pair keeps the
ceremony usable for anyone who registers their own OAuth application.

Corollary: the error message is the documentation. It must always name both
paths, never just fail.

## 2. A login row exists only when a wire can spend the credential

`LOGIN_FLOWS` lists `anthropic`, `openrouter`, `xai` — the providers where a
stored token/key actually reaches a request today (Anthropic's wire switches to
Bearer + Claude Code headers on an `sk-ant-oat` key; xAI's `toAuth` is
`{apiKey: access}` on the OpenAI-compatible wire; OpenRouter's exchange mints a
permanent API key).

`openai-codex` is deliberately absent: it needs a subscription base URL (the
ChatGPT backend) with no catalog rows or wires here, and its model list is not
published anywhere the port can verify against (`docs/plans/018-…`). Storing a token
nothing can spend is worse than an honest gap.
`epsilon-llm/tests/oauth_login.rs::login_flows_only_advertise_wires_that_exist`
pins that promise — update the test the day a wire lands, not the other way
around.

**`github-copilot` joined at M55** by exactly that route: its wire
(`CopilotWire`), catalog rows (models.dev's `github-copilot` registry), the
device flow, and the mint that turns a GitHub token into a Copilot bearer all
landed before the login table did. Its client id resolves like every other
(`GITHUB_COPILOT_OAUTH_CLIENT_ID` / `[oauth.github-copilot] client_id`), and
Business/Enterprise accounts additionally read
`[oauth.github-copilot] enterprise_domain`, which is stored **with** the
credential: the wire and every later refresh must keep talking to the host the
token was minted for, even if the config that named it is gone.

## 3. A `real` feature must forward to every crate that gates real code

The bug M52 uncovered: `epsilon-cli`'s `real` feature enabled only
`epsilon-llm/real`, so `epsilon-core/real` stayed off and
`Agent::try_hot_swap_to_real` compiled to its `false` stub in every installed
binary — the TUI advertised a live hot-swap it could not perform.

Rule: a feature that names a capability forwards to **every** crate whose code
implements it (`real = ["epsilon-llm/real", "epsilon-core/real", "epsilon-tui/real"]`),
and a compile-time marker makes an omission loud:
`epsilon_core::REAL_PATH_ENABLED` is bound in `epsilon-cli` under the same
feature, so a build that lights the network layer without lighting core's real
code fails to compile instead of degrading silently.

## 4. Naming: the ceremony argument is not the token shape

`oauth_login::is_oauth_ceremony_arg(value)` answers "is this CLI token the word
`oauth`"; `providers::anthropic::is_oauth_token(key)` answers "does this key
look like an OAuth access token". The first draft reused the second name and
the reviewer's confusion was immediate — keep them distinct.
