# M46 — pi-parity: OAuth credential shape (`auth/types.ts` storage half)

Fourteenth port batch against the 2026-09-03 parity audit (providers/auth
findings — OAuth storage). The credential store only knew api keys: a pi
`auth.json` containing `{type:"oauth", refresh, access, expires}` failed to
parse, and there was no API to save a token pair or read a live access token.
Full OAuth *flows* (PKCE login, refresh exchange) are deliberately out of
scope — see below — but the storage half is now pi-shaped.

## What landed

1. **Enum store** (`epsilon-llm/src/auth.rs`): `StoredCredential` is now
   `Api { key, env? }` / `Oauth { refresh, access, expires }` with
   `kind()`/`key()`/`env()` accessors. Custom `Serialize` keeps the on-disk
   shape exactly opencode's `{type:"api", key, env?}` (roundtrip test still
   green); `Deserialize` goes through `StoredCredentialWire`, which also
   accepts pi's `{type:"api_key"}` (coerced) and `{type:"oauth"}` (extras
   ignored). Unknown discriminators still fail — corrupt is corrupt.
2. **OAuth API**: `save_oauth[_in]` (locked, atomic, 0600, no-clobber, empty
   refused), `oauth_access_token[_in]` returning `OauthToken::Live/Missing/
   Expired` with a 60s skew margin (expired tokens are REPORTED, never sent).
3. **Call-site updates**: `resolve_api_key` only resolves `Api` (oauth never
   becomes an api key); `epsilon auth` list shows `oauth (expires …)`.
4. **Tests**: `oauth_and_legacy_api_key_shapes_read` (oauth/extras/legacy/
   unknown) + `oauth_save_expiry_and_token_states` (live/expired/missing/
   refusal). Auth suite 13/13.

## Key decisions

- Storage, NOT flows: pi's `oauth/` directory is ~1000 lines of
  provider-specific login ceremonies (PKCE callback servers, device-code
  polling, per-provider token endpoints + client ids) plus a refresh
  scheduler inside `Models`. Porting the *ceremonies* without the interactive
  login UX (`epsilon auth login --oauth` doesn't exist yet) would be dead
  code with live secrets. The store accepts what flows will produce; the
  flows come with the login command (tracked, not forgotten).
- No refresh exchange yet (`refreshAnthropicToken` equivalent): refresh needs
  per-provider token URLs + client ids (pi hardcodes them per file) and
  network access under the store lock. `OauthToken::Expired` is the seam —
  the refresh call will sit exactly where `Expired` surfaces.
- Writes normalize (`api_key` → `api`): files converge instead of
  accumulating legacy spellings. A pi-authored file with `api_key` entries
  heals on next save.

## Gotchas

- `#[serde(tag = "type")]` on the enum serializes `Api` as
  `{"type":"Api", ...}` (variant name, not `"api"`) — broke the opencode
  roundtrip test. Fix: `Deserialize`-only derive + hand-written `Serialize`
  (custom `SerializeMap`), so reads are liberal and writes are canonical.
- Four auth tests used struct field syntax (`.key`, `.kind`) — bulk-updated
  to accessors via script, verified counts per site.
- Two patch operations ate each other's context (`remove_credential_in` lost
  its `let path` line, then its `fn` line) — recovered by re-reading the
  region and re-adding both lines. Same standing discipline: re-read before
  retrying stacked patches on one function.

## Verification

- `cargo test -p epsilon-llm --lib auth`: 13 passed (2 new).
- fmt clean, workspace clippy zero.
