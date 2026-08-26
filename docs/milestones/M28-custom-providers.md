# M28 - Custom OpenAI-compatible providers

Date: 2026-08-26. Closes a provider-wiring gap both audited references cover
and we did not: pointing Epsilon at endpoints that are not in the bundled
catalog (LM Studio, vLLM, llama.cpp server, a fresh cloud provider models.dev
has not listed yet).

## What

A new optional section in the same global config file
(`~/.config/epsilon/config.toml`):

```toml
[provider_custom.lmstudio]
base_url = "http://localhost:1234/v1"
# env_key = "LMSTUDIO_API_KEY"   # optional; omit for keyless local servers

[provider_custom.together]
base_url = "https://api.together.xyz/v1"
env_key = "TOGETHER_API_KEY"
```

Model ids prefixed with that name now stream to your endpoint:

```sh
epsilon --provider real --model lmstudio/qwen2.5-7b-instruct -p "hi"
target/release/epsilon --model together/mixtral-8x7b -p "summarize this repo"
```

Rules:

- Known catalog ids always win. `openai/gpt-4` routes by its catalog row even
  if someone declares `[provider_custom.openai]`; a custom row can never
  shadow a real provider.
- The id form is `<name>/<model>`; `<name>` is case-insensitive.
- Keys resolve exactly like built-in providers: stored
  `epsilon auth login <name>` entry first, then the row's `env_key`. No key at
  all is fine for local servers (no auth header sent).
- Rows without a usable `base_url` are skipped silently; a half-written entry
  must not take the whole catalog down.
- The startup diagnostic names the source correctly for customs too
  (`key stored (epsilon auth login)` vs `key env var`).
- Fallback chains accept custom ids as candidates (`--fallback lmstudio/foo`).

## Attribution

opencode lets every provider carry `options.baseURL` /
`options.endpoint` (`packages/opencode/src/provider/provider.ts:361`); Hermes
supports `base_url_override` per route. Both ideas land here as one feature:
user-declared endpoints behind our existing catalog-driven routing seam.

## Key decisions

1. **One parser, next to the data's consumer.** First draft put the TOML
   parsing in epsilon-core's config module. That broke architecture law the
   moment wire routing needed the type: epsilon-llm depends only on
   epsilon-contracts, so referencing a core type from the provider layer is an
   illegal upward edge (the compile-enforced dependency rule caught it, as
   designed). Final shape: `epsilon_llm::custom` owns BOTH the type and the
   parsing, using the same `auth::global_dir()` the credential store already
   uses. Config stays declarative; llm grows no dependencies beyond the
   workspace-pinned `toml`.
2. **De-static the OpenAI-compat wire, not the catalog.** `WireApi::OpenAiCompletions`
   carries `&'static str`; widening the catalog to owned strings would churn
   every generated row for one feature. Instead `OpenAiCompatWire` fields went
   from `&'static str` to `String`, and the `ProviderWire::env_key` trait
   method returned `&'static str` -> `&str`. Catalog rows convert with
   `.to_string()` at wire construction; customs pass their values through.
3. **Load customs at call time, not process start.** Three call sites read
   the config file when they need it (two stream paths + fallback). It is one
   small TOML parse per stream attempt on the REAL path only; faux/offline
   builds never touch it. No global state to keep coherent.
4. **No picker UI changes.** The picker's synthetic "custom id" row already
   accepts any typed text; footer labels fall back to the raw id honestly;
   context/cost simply show nothing for unknown ids instead of guessing.
   Customs ride existing UX for free.

## Test-caught bugs

None external, but the layering violation above was caught by cargo before it
could become an architectural debt — worth recording because the fix (moving
the type into epsilon-llm) produced a better design than the original plan.

## Verification

```
cargo fmt --all -- --check               clean
cargo clippy --workspace --all-targets   0 warnings
cargo test --workspace                   249 passed, 0 failed
cargo test --workspace --features real   264 passed, 0 failed
cargo run -q -p epsilon-cli -- --demo    completes
cargo check -p epsilon-cli --features real   compiles
```

New tests:

- `custom.rs`: normalization/rejection, prefix matching, TOML loading
  (sorted, tolerant of broken rows, leaves other keys alone), missing-file.
- `wire_fixtures.rs`: `custom_provider_id_routes_to_declared_endpoint`
  (endpoint join, empty env_key means no auth header, prefix lookup).

## Docs

README (status row, totals, Real providers section), CHANGELOG, glossary term
"Custom provider", ADR-003 amendment (behavior-level: routing accepts
config-declared endpoints).
