# 09 — Adding a provider whose host, auth, or refresh deviates

Written while porting GitHub Copilot (M55), the first provider that broke three
assumptions the earlier fifteen providers never touched. Each deviation has
exactly one extension point — use them instead of special-casing the transport.

## 1. The host depends on the credential → `ProviderWire::endpoint_for`

`endpoint(model)` sees the catalog row, never the key. Copilot's bearer carries
`proxy-ep=<host>` and the API host is derived from it, so the URL cannot be
static. The trait method to override is:

```rust
fn endpoint_for(&self, model: &str, key: &str) -> String {
    self.endpoint(model) // default: static hosts keep working untouched
}
```

Both request sites (`try_stream_direct` and the streaming path) call it with the
resolved key. Anything that needs the credential to build a URL — tenant hosts,
regional endpoints, `proxy-ep` — belongs here, not in a match arm on the
provider name inside `providers::mod`.

## 2. The provider is a proxy for other families → wrap, don't fork

Copilot re-serves Claude over Anthropic Messages and everything else over the
OpenAI-compatible surface. The port wraps the family wire and overrides only
what differs (URL + auth headers), delegating `request_body`, `on_event`,
`map_stop`, and `finalize_usage_from_tail`. A second copy of a parser is a
maintenance liability: a payload fix in `anthropic.rs` would silently miss the
proxy.

Rule of thumb: **override the minimum surface, delegate the protocol.**

## 3. Refresh is not a token rotation → `RefreshStyle`

pi's three shapes are modelled as an enum, not flags:
`JsonBody` (Anthropic's `postJson`), `FormBody` (xai/codex), and `CopilotMint`
(GET, editor headers, no client id, no body, `{token, expires_at}` back, and the
*refresh token is re-spent rather than rotated*). Adding a fourth shape means a
new variant plus its parser — never a `bool` argument through
`exchange_refresh_token`.

Corollary for storage: a refresh that does not rotate must keep the stored
refresh token, and `store_refreshed_oauth_in` carries credential extras forward
because they describe the account, not the pair.

## 4. Provider extras belong to the credential

`StoredCredential::Oauth` carries an optional `env` map for values a login mints
beside the pair (Copilot's `enterprise_domain`; later: project/location-shaped
data). Config can change or disappear after a login, and the wire must keep
talking to the host the token was minted for, so anything the *request* needs
travels with the credential.

## 5. Catalog rows: prefix ids the native providers already claim

A registry that re-serves another provider's model ids (`github-copilot` serves
`claude-opus-5`, `gpt-5.4`) must prefix them (`github-copilot/claude-opus-5`) —
the generator's first-provider-wins dedupe would otherwise drop the whole
registry as duplicates. `vertex/`, `bedrock/`, and `github-copilot/` exist for
exactly this reason, and the prefix is chosen per registry in
`scripts/regen_catalog.py` (short form: `vertex/`, not `google-vertex/`).

## 6. Never store a credential the wire cannot spend

`LOGIN_FLOWS` only lists providers with a wire + catalog rows that can spend the
token; a test pins the list so the gap cannot spread silently. Copilot waited
for M55 for exactly this reason, and `openai-codex` is still out because its
model data is not published anywhere this repo can verify against.
