# M22 — opencode auth store + interactive trust re-prompt

Date: 2026-08-25. Two user-facing quality-of-life gaps from the standing loop:
the setup experience ("the way opencode lets the user set up his API and model
is way better" — direct instruction) and DIRECTION-01 (trust re-prompt, the
audit program's top remaining recommendation).

## Slice 1: `epsilon auth` — opencode's credential UX (opencode auth.json port)

Before: Epsilon only read API keys from environment variables. A beginner had
to learn what an env var is, how to set one per shell, and why it vanished in
a new terminal — exactly the friction opencode removed.

After: keys live once in `~/.config/epsilon/auth.json` and every later session
just works:

```
epsilon auth login anthropic          # prompts for the key (not in shell history)
epsilon auth login deepseek sk-...    # scripting: pass it inline
epsilon auth list                     # providers + masked keys (…last4 only)
epsilon auth logout anthropic         # remove
```

Design decisions:

- **File + schema match opencode** (`auth.json`, `{provider: {type:"api",
  key}}`) so the mental model transfers 1:1; 0600 perms on unix, profile ACL
  on Windows.
- **Precedence is stored-wins, env-fallback** (`catalog::resolve_api_key`):
  `auth login` is the documented path, but env vars keep working for CI and
  users who prefer them. An empty stored key counts as absent.
- **Placement: epsilon-llm/src/auth.rs, NOT core.** The interrupted first
  attempt put the store in epsilon-core and had catalog (in llm) call into it
  — an impossible direction under our dependency law (`contracts <- llm <-
  core`) that broke the whole build. pi keeps provider auth inside its
  `packages/ai` brick; we now do the same.
- **Everything downstream follows automatically**: `credentials_ready`
  (auto-real detection), the picker's auth column, live `/models` fetching,
  and DirectProvider request auth all go through `resolve_api_key`.
- Startup banner now says where the key came from (`key stored
  (epsilon auth login)` vs `key env var`) and the offline banner points at
  `epsilon auth login` as the way to go real.
- `list` masks everything but the last 4 chars — screenshots can't leak keys.

## Slice 2: trust re-prompt — DIRECTION-01 closed (pi trust-manager semantics)

ADR-012 already recorded a SHA-256 hash of `.epsilon/config.toml`; a CHANGED
config printed a loud banner **but still ran**. That was the gap: silently
changed project rules (which can tighten or loosen permissions) deserved an
explicit decision before anything executes.

Now:

- `load_project_config` returns an explicit [`TrustDecision`] (Absent /
  Trusted / Unchanged / Changed) instead of burying the verdict in strings.
- Headless modes keep the loud-banner policy (still loads; automation must
  not break).
- The interactive TUI **holds all submits** until the user answers:
  type `trust` to accept (session continues) or `trust no` to quit. Anything
  else gets bounced with guidance — nothing reaches the model against
  unreviewed rules. The old hash stays on disk until acceptance, so every run
  re-reports until someone decides (no silent re-trust).

## Test-caught bugs / gotchas

- **The architecture-law violation above**: the session before this one died
  mid-slice with the workspace not compiling (`crate::config::load_credentials`
  inside llm). Fix = move the store to llm, delete core's copy, keep core's
  serde dep removal honest.
- My own M21 code had a latent instance of the notices-channel bug: bare
  `/think` used the async channel instead of a direct push. The new trust
  test caught it because I reused the same assertion pattern. Lesson stands:
  slash-command replies push DIRECTLY into UiState.
- First draft of `read_secret_line` had an `unsafe extern "system"` stub —
  workspace forbids unsafe; replaced with plain stdin line-read and an honest
  doc comment (echo suppression deferred until actually needed).
- `env::set_var` is unsafe in edition 2024 → cannot be called in tests under
  `unsafe_code=forbid`. Rewrote the test to pin the write-side guard instead
  (blank keys are rejected at save time).

## Verification

Full battery (both feature configs): fmt clean · clippy 0 warnings ·
offline tests green · real-feature tests green · demo completes ·
`epsilon auth list` smoke-tested live.

## Known limits / next

- Key entry echo is not suppressed yet (plain stdin read). Acceptable for v1;
  a crossterm-based hidden reader is the follow-up if it bothers anyone.
- No OAuth flows (opencode has them for Copilot/xAI); our wires are all
  static-key today — revisit when a wire actually needs OAuth.
- Trust review covers project config; extensions/MCP declarations ride the
  same file so they inherit the protection.
