# M20 — Model catalog regeneration + live picker refresh

Date: 2026-08-24. Trigger: the user reported the model picker "still shows old
models" and pointed at opencode/Hermes behavior as the reference. This milestone
closes plans/README.md's DIRECTION-05 (catalog regenerate script) and imports the
opencode/Hermes live-listing convention into the picker.

## The problem, precisely

The bundled catalog was a hand-curated ~40-row snapshot from early August.
Models released since (claude-opus-4-7/4-8, gpt-5.5/5.6 family, gemini-3.7-flash,
glm-5.1/5.2, kimi-k3, ...) were invisible unless the user typed their id by hand.
Worse, the M17 "live fetch" escape hatch (Ctrl+F) was dead on arrival:

1. **Wrong argument**: the TUI passed a MODEL ID (`row.id`) to a fetcher whose
   implementation (`catalog::live_models`) matches on PROVIDER names — every call
   errored with `no live listing endpoint for provider 'gpt-5.4'`. A doc comment
   claimed the model-id contract, so the bug survived review.
2. **Invisible even when it worked**: results landed in a flat `live_cache` that
   only fed the legacy single-stage list — but M16c replaced that view with the
   two-stage picker, whose stage 2 renders from `build_models_for_provider`,
   which never read the cache.

Two lessons recorded: a stale doc comment can *cause* a bug by mis-teaching the
contract; and a cache wired to a view nobody renders anymore is silent dead code
— grep for consumers when a UI is restructured.

## What shipped

### 1. Generated catalog data + regen script (DIRECTION-05)

`scripts/regen_catalog.py` (stdlib-only) fetches <https://models.dev/api.json> and
writes `crates/epsilon-llm/src/catalog/catalog_data.rs` — a generated
`CATALOG_ROWS` table replacing the hand-written one. Selection rules, documented in
the script:

- nine direct-API providers Epsilon routes wires for;
- `tool_call == true` and text-only output (this harness streams text+tools);
- dated snapshot ids (`...-20251001`) excluded in favor of canonical aliases;
- advertised context window must be > 0;
- first-provider-wins dedupe (groq hosts `openai/gpt-oss-*` under ids identical to
  openrouter's vendor-prefixed ones — direct API beats aggregator so routing stays
  unambiguous; the loser is logged).

Prices keep the models.dev ×1e6 micro-USD convention. Unpriced rows pass through
as zero where upstream genuinely lists none (google gemma previews); we render
what the registry states, never invented rates. Result: **383 rows** (10 anthropic,
37 openai, 5 deepseek, 18 google, 6 xai, 6 groq, 10 moonshot, 14 zai, 277
openrouter) plus the fixed faux/ollama offline rows. Refresh loop:
`python scripts/regen_catalog.py && cargo fmt && cargo test -p epsilon-llm`, fix
moved pins, commit script+data together.

`catalog.rs` becomes a thin accessor layer: `mod catalog_data`, wire consts, the
`ModelInfo` struct, accessors (`curated`/`lookup_model`/`display_label`/
`credentials_ready`/`env_key_for`), the live-listing endpoint table, and tests.
Tests updated: price guard now asserts every cloud PROVIDER has ≥1 priced row
(placeholder regression guard) instead of demanding every row be priced;
`catalog_is_current_generation` pins the newest flagship per family at regen time
plus negative pins for gone/dated ids.

### 2. Live listing actually reaches both picker stages

- `catalog::live_listing_available(provider)` — static, ungated knowledge of which
  providers expose `GET /models`, so the TUI can decide before injecting a
  `real`-feature fetcher. `listing_endpoint` extracted from `live_models`.
- `LiveFetcher` keeps its signature but its documented contract (and actual use)
  now passes the provider name. Fixed at the single call site via the shared path.
- `App::request_live(force)` replaces `fetch_live_for_selected`: stage-aware target
  resolution (stage 2's provider, else highlighted stage-1 row), once-per-session
  dedupe via `live_fetched`, one-fetch-in-flight guard. `force=true` (Ctrl+F)
  retries and surfaces failures; auto-fetch is quiet on failure and never refetches.
- Auto-fire hooks: `/models` open, Ctrl+M open, and Enter-descent into a provider
  all call `request_live(false)` (opencode/Hermes convention: the list you are
  looking at refreshes itself; bundled data remains instant fallback).
- `live_cache` is now per-provider `(provider, Vec<id>)`; the fetched channel
  carries `(provider, result)` so concurrent fetches cannot cross-label results.
- `build_rows` groups unknown live ids under their own provider;
  `build_models_for_provider(provider, ..., live)` merges that provider's live ids
  into stage 2 with fuzzy filtering and no duplicates against catalog rows.

### 3. Clippy debt under --features real

M19's fixture-visibility widening left 11 warnings visible only under
`clippy --all-targets --features real` (a lane no gate covered). Fixed: missing
docs on the three wire structs and `LineReassembler::new`, collapsible ifs in the
OpenAI-compat delta handler, `Usage` built via initializer instead of field
assignment, `Copy`-type `.clone()` removals in usage selection, `new_without_default`
allow on test-support. Workspace clippy is now zero under BOTH feature configs.

## Test-caught bugs / gotchas

- My first negative pin used an invented id (`claude-opus-4-8-20260528`) instead
  of reading the skip log; the real dated ids come from the generator output.
  Fix the test from evidence, not imagination.
- The old test asserted `gpt-4o` was ABSENT ("stale row") while today's registry
  still lists it — the curated table had been taste-filtered. Policy now explicit:
  registry-faithful, not taste-curated; the pin flipped to presence.
- Python heredoc edit renamed away a `def render` line (script broke); rewritten
  wholesale rather than patch-stacked.
- Borrow checker: moving `provider` into `live_cache` then inserting it into
  `live_fetched` needed an explicit clone at one site (cheap, obvious).

## Verification

```
cargo fmt --all -- --check                    → clean
cargo clippy --workspace --all-targets        → 0 warnings
cargo clippy ... --features real              → 0 warnings (new coverage)
cargo test --workspace                        → all green (see README total)
cargo test -p epsilon-llm --features real     → 30 green (24 unit + 6 fixtures)
cargo run -q -p epsilon-cli -- --demo         → completes
cargo check -p epsilon-cli --features real    → ok
```

New tests: stage-2 live merge (provider-scoped, no dupes), build_rows provider
grouping assertion extended.

## Known limits / next

- OpenRouter's live `/models` returns paginated JSON; we read page one (same as
  before). Fine for discovery — full pagination is future work if needed.
- Auto-fetch fires for the highlighted stage-1 provider only; typing a filter does
  not fan out fetches across every matching provider (would multiply requests).
- Catalog freshness still requires running the regen script manually; CI could
  open a weekly drift PR later (recorded as maintainer option).
