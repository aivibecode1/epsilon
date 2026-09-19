# M52 — pi-parity: OAuth login ceremonies + the `real`-feature forwarding fix

Twentieth port batch against the 2026-09-03 parity audit. M46 built the OAuth
credential shape, M51 consumed it (refresh-before-use, OAuth-first resolution
for `anthropic`) — and both left the same sentence in the log: *"No login
ceremonies: PKCE callback servers and device-code polling still wait for
`epsilon auth login --oauth`."* This milestone is that waiting workstream, plus
a bug it uncovered on the way in: `--features real` never lit
`epsilon-core/real`, so the TUI's live hot-swap had been compiling to its
`false` stub in every installed binary.

## What landed

1. **`epsilon-llm/src/oauth_login.rs`** (new; pi `packages/ai/src/auth/oauth/*.ts`
   login half):
   - **PKCE** (`pkce.ts`): [`generate_pkce`] over the OS CSPRNG (`getrandom`),
     `base64url` challenge (RFC 7636 Appendix A encoding), S256; `random_state`
     for the `state` parameter.
   - **Paste parsing** (`anthropic.ts::parseAuthorizationInput`): full redirect
     URL, `code#state`, bare query string, or bare code — the four shapes a
     user can produce when the browser cannot reach the loopback listener.
   - **Callback listener**: a one-shot `127.0.0.1` HTTP server (std::net, no new
     dep) that answers the browser with a small HTML page, enforces the `state`
     match, and hands the `(code, state)` pair to the flow exactly once.
     Fixed port for Anthropic (53692), ephemeral for OpenRouter, random path
     suffix on ephemeral flows so a stale tab cannot post into a fresh
     listener.
   - **Device flow** (`device-code.ts`, RFC 8628): generic poller with pi's
     semantics — 1s minimum interval, wait before the first poll, `slow_down`
     honours a server interval else adds 5s, and the timeout message
     distinguishes a plain timeout from "saw slow_down, usually WSL/VM clock
     drift".
   - **Exchanges**: PKCE code exchange (JSON for Anthropic, OpenRouter's
     `{code, code_verifier, code_challenge_method}` → permanent `key`), device
     authorization + token polling (form-encoded), each with a telling error
     and a 30s timeout. All `real`-gated; every parser is always compiled and
     offline-tested.
   - **Orchestrator** `login(provider, ui, allow_callback)`: synchronous,
     drives its own current-thread runtime, so the CLI can call it before its
     main runtime exists and the TUI can call it from a worker thread.
2. **Provider table** — three rows Epsilon's wires can actually spend today:
   `anthropic` (PKCE, Claude Pro/Max), `openrouter` (PKCE → API key),
   `xai` (device flow, SuperGrok / X Premium). `openai-codex` and
   `github-copilot` are deliberately absent: pi reaches them through
   subscription base URLs (ChatGPT backend, Copilot proxy) that Epsilon has no
   catalog rows or wires for, and storing a token nothing can spend would be
   worse than an honest gap. A test pins that promise.
3. **Client ids: env or config, never embedded.** Resolution order is
   `<PROVIDER>_OAUTH_CLIENT_ID` → `[oauth.<provider>] client_id` in the global
   `config.toml` → a telling error naming both. Unchanged from M46/M51: pi
   embeds its own per provider (Anthropic's is base64-obfuscated); Epsilon will
   not ship another project's credential.
4. **CLI**: `epsilon auth login <provider> --oauth [--paste]`. The listener and
   a pasted redirect race for the code (a stdin reader thread feeds a channel,
   polled alongside the callback), so a browser on another machine still works;
   `--paste` skips the listener entirely. `auth list` already prints the pair's
   expiry.
5. **TUI**: `/login <provider> oauth` runs the ceremony on a worker thread;
   progress arrives as notices, the code paste uses the existing masked overlay
   (opened by an `OauthUiEvent::NeedCode` message, closed by `Done`), and Esc
   drops the code sender — which is how the worker sees a cancellation.
   Discoverability: the key prompt for an OAuth-capable provider advertises
   `Ctrl+O = subscription login` (and Ctrl+O switches straight into the
   ceremony), and the bare `/login` provider picker marks those rows with
   `⌁ oauth`.
6. **`xai` joins the OAuth-first resolution** in `catalog.rs` (pi
   `xaiOAuth.toAuth` returns the access token as the bearer key; the refresh row
   already existed from M51). `openrouter` needs no branch: its exchange stores
   a plain API key through the Api path.
7. **The feature-forwarding fix.** `epsilon-cli`'s `real` feature lit only
   `epsilon-llm/real`, which left `epsilon-core/real` off; `Agent::try_hot_swap_to_real`
   therefore compiled to its `#[cfg(not(feature = "real"))]` stub and the TUI's
   "now live on X" path degraded to "Restart epsilon" in the installed binary
   (M32d's fix could not have worked as documented). All three crates now
   forward (`epsilon-llm/real`, `epsilon-core/real`, `epsilon-tui/real`), and a
   compile-time guard (`epsilon_core::REAL_PATH_ENABLED` bound in the CLI) makes
   the omission a build error instead of a silent downgrade.
8. **`getrandom = "=0.3.4"` + `base64` become direct deps** of `epsilon-llm`
   (both already in the lock transitively). PKCE's verifier must be
   cryptographically random (RFC 7636 §4.1) and base64url-encoded; std has no
   RNG, and hand-rolling one would be strictly worse than the audited
   OS-randomness shim.

## Key decisions

- **Sync orchestrator, not async.** The ceremony spans blocking user input
  (paste) and network IO. Making it async would force the CLI to build its
  runtime earlier than it wants and the TUI to hand a runtime into a worker
  thread. A sync function with an internal current-thread runtime keeps both
  callers trivial and keeps `LoginUi` implementations free to block.
- **Listener + paste race, not listener only.** pi runs a callback server and
  falls back to a manual prompt; the CLI here polls both concurrently. The
  alternative (listener only) strands ssh/remote users for the full deadline.
- **Flow rows require a spendable wire.** See #2 above — the table is a
  capability statement, not a wish list.
- **`OauthUiEvent::NeedCode` rather than "the thread pokes the App".** The
  worker never touches `App`; the two channels (`notices_tx`, `oauth_tx`) are
  the whole interface, so the flow is testable without a terminal (the new TUI
  tests drive the state machine directly).
- **Marker const for feature forwarding.** A comment cannot fail a build;
  `epsilon_core::REAL_PATH_ENABLED` referenced from `epsilon-cli` can.

## Gotchas (each one cost a real test failure)

- **Windows accepted sockets inherit non-blocking mode.** The callback
  listener's single `read()` came back `WouldBlock` on some runs, so the
  request line parsed as empty and the browser got "400 malformed request
  line" instead of the success page. Fixed with a bounded read loop that
  retries `WouldBlock` until the request line arrives (500ms ceiling). The
  first version of the test passed only because the data usually arrives
  before `accept` returns.
- **URL fragments are not query parameters.** `parse_authorization_input` glued
  `#fragment` onto the last parameter's value; a redirect URL that carries a
  fragment produced `state = "xyz#fragment"` and a spurious state mismatch.
  pi's `new URL(value)` drops the fragment, and now so do we.
- **Field order is part of an error contract.** `parse_device_code` reported
  "missing verification_uri" for a body that was also missing `user_code`; pi
  reads fields in declaration order, so the *first* missing field is what a
  user should see. Reordered to match.
- **`epsilon-tui`'s real feature was itself never enabled** by the CLI (see
  #7) — which is why the TUI's new `#[cfg(feature = "real")]` login path is
  reachable in the shipped build only after the forwarding fix.

## Verification

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets`: zero warnings (default lane).
- `cargo test -p epsilon-llm --test oauth_login`: 23 new tests, all green —
  RFC 7636 Appendix B S256 vector, PKCE self-consistency + charset, paste
  shapes incl. fragment/percent-decoding, authorize-URL parameter set for both
  PKCE providers, callback listener (capture, state mismatch kept listening,
  404 route, timeout, state-less flow), callback request-line parsing, device
  poller (pending → slow_down → complete, server interval honored as an exact
  gap, drift message, terminal failure), device response parsing
  (untrusted URI refused, first-missing-field order), exchange parsing (pair
  skew, OpenRouter key, missing refresh token), config client-id resolution,
  provider-table promise.
- `cargo test -p epsilon-tui --lib`: 134 passed (4 new: overlay open/close,
  Enter hands the code over + Esc disconnects, `oauth` never read as a key,
  Ctrl+O switches the key prompt into the ceremony).
- `cargo test --workspace`: green (see the history row for the count).
- `cargo check -p epsilon-cli --features real`: clean, and `cargo tree` confirms
  `epsilon-core feature "real"` is now on in that build.
- `cargo run -q -p epsilon-cli -- --demo`: completes.

## Follow-ups (not gaps in this rung)

- `openai-codex` / `github-copilot` logins need their subscription wires and
  catalog rows first; `github-copilot` additionally needs a refresh shape that
  exchanges the GitHub token for a Copilot bearer (its `refresh` is not the
  standard pair rotation).
- The picker badges the oauth-capable rows and Ctrl+O jumps in, but a bare
  Enter still opens the key prompt; a two-option selector (key vs subscription)
  would be the next polish rung.
- `epsilon auth login <provider> --oauth` on a build without `real` prints the
  rebuild line; a shipped artifact would carry the feature.
