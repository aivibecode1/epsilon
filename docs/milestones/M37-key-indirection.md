# M37 — pi-parity P1 batch 1: key indirection (`$ENV`/`${ENV}`/`!command`)

Fifth port batch against the 2026-09-03 parity audit (session/auth/config
finding S14). A stored `auth.json` key was a literal or nothing: pi's
`AuthStorage::read` instead resolves it through `resolveConfigValue`, so keys
can live in secret managers and env templates instead of on disk.

## What landed

1. **New `config_value` module** (`crates/epsilon-llm/src/config_value.rs`):
   pi `resolve-config-value.ts` at the value layer — literals pass through,
   `$VAR`/`${VAR}` templates resolve against the credential's `env` map then
   the process environment (pi's order), `$$`/`$!` stay escapes, invalid names
   (`${not-a-name}`) and dangling `$` stay literal, `!shell commands` run once
   (10s timeout, stderr discarded, stdout trimmed) and cache per process.
   Failures return `None` — the caller falls through, never panics.
2. **`resolve_api_key` honors indirection.** Stored keys resolve through the
   module with their per-credential `env` map; unresolvable values fall
   through to the provider env var (a dangling `$MISSING` degrades to the old
   lookup instead of failing auth). `!command` values are deliberately NOT
   executed on this path (pi's `isCommandConfigValue` guard): spawning a
   secrets helper from a key *read* would fire from UI/list paths too.
3. **Credential shape**: `StoredCredential` gains optional `env`
   (`BTreeMap`, `skip_serializing` when absent — existing files parse
   unchanged); `save_credential_full_in` persists it, plain
   `save_credential_in` delegates with `None`.

## Key decisions

- No `!command` execution in `resolve_api_key` *yet*: pi resolves commands on
  read too, but only after the read path is the single chokepoint. Epsilon's
  read path is shared with display/diagnostic callers; executing there would
  run helpers from UI code. Commands resolve inside `config_value` (tested)
  and wire up to auth reads in a follow-up once call sites are audited.
- No new dependency: template parsing is ~60 lines of byte scanning; process
  spawn is `std::process::Command` (PowerShell on Windows — the bash-tool
  precedent — `sh -c` elsewhere).
- No `set_var` in tests: edition-2024 `set_var` is `unsafe` and the workspace
  forbids `unsafe_code` outright, so env cases go through the `extra_env` map
  (the store's own code path) plus guaranteed-present `PATH` for the
  process-env fallthrough.

## Gotchas

- First draft mutated process env in tests; the workspace `unsafe_code =
  "forbid"` lint rejects even test `unsafe` blocks — restructured to map
  injection instead (strictly better: no global mutation, no serial-test
  hazard).
- Two clippy nits from the new code (`collapsible_if`, identical `cfg!`
  branches invented while writing portable command tests): collapsed to
  let-chains and branch-free literals immediately — zero warnings restored.

## Verification

- 7 new `config_value` tests (literals, `$`/`${}` via map + process env,
  map-shadows-process, missing-var `None` + reported name, escapes/invalid
  names literal, command trim + cache, failing-command `None`).
- New `catalog` test `resolve_api_key_honors_key_indirection` (template via
  credential env resolves, dangling template falls through to provider env,
  `!command` never executes).
- Full `cargo test -p epsilon-llm --features real --lib`: 50/50.
- `cargo fmt --check`: clean. `cargo clippy -p epsilon-llm --features real
  --all-targets`: zero warnings.
