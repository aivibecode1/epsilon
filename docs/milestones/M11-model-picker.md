# M11 — Model picker & catalog + skills/extensions runtime wiring

Date: 2026-08-23 · Status: shipped. M12 (config layer completion, reliability stack,
rpc/OpenAPI, Linux verification) follows.

## What was built

- **Bundled catalog** (`epsilon_llm::catalog`): 18 models across 10 providers
  (faux, anthropic, openai, deepseek, google, xai, groq, moonshot, openrouter,
  ollama-local) with `{id, provider, label, context_window, max_output, placeholder
  costs, supports_tools, env_key}`. Extends the M3-era three-entry placeholder; the
  capability-fields-not-substrings rule survives. `credentials_ready(id)` /
  `env_key_for(id)` power auth hints. pi's generated-catalog approach — offline browse,
  no keys needed; registry sync stays additive.
- **Config slice** (`epsilon_core::config`): `[model] id` in
  `~/.config/epsilon/config.toml`. `_in(dir)` variants make it testable without env
  mutation (Rust 2024 made `set_var` unsafe and core forbids unsafe). Reads fail open;
  writes preserve unknown sections and refuse blank ids / non-table `[model]`.
- **TUI picker**: `/models` opens a filter-driven dialog — type-to-filter over
  id/label/provider, provider group separators, active model floated to top with ●
  marker, 🔑/· auth column, custom-id row for anything unknown (genai routes arbitrary
  ids by name). Enter switches via `Agent::set_model_id`, persists through config, and
  posts a transcript notice. `/model <id>` direct; `/help`; footer shows the active
  model label.
- **Precedence** (ADR-018): `--model` flag (ephemeral) > saved config > `"faux-1"`,
  applied in every mode at dispatch.
- **Skills + extensions wiring** (closes the audit's reachability debt from M8):
  `build_runtime` now discovers skills under `~/.config/epsilon/skills` +
  `.epsilon/skills` into the CONTEXT-tier catalog, and spawns extension hosts from
  `~/.config/epsilon/extensions` + `.epsilon/extensions/<dir>/extension.json`,
  registering tools namespaced `ext__<name>__<tool>` (collision-checked) and pumping
  broadcast events through each host's manifest filter (`send_event(true, …)` — hosts
  filter internally per the M9a contract). Broken manifests/duplicates report as
  diagnostics without blocking startup.
- **`--list-models [provider]`**: headless catalog table with key-status column.

## Test-caught notes

- PowerShell-style string surgery corrupted a match arm mid-edit (duplicate fragment);
  caught by the compiler as delimiter mismatch. Re-affirms the AGENTS.md Windows note:
  prefer real file-edit tooling over bulk replace in this repo.
- `--list-models <provider>` initially died with "unknown argument": my arm called
  `positional()` before advancing `i`, so it inspected its own flag token instead of the
  next argument. Every other flag advances first — convention exists for a reason.
- Rust 2024 `set_var` is unsafe: config tests would have needed unsafe blocks;
  refactoring to explicit-dir functions (`load_model_id_in`) removed the need entirely —
  better API anyway.
- toml crate's `Entry::or_insert_with` takes a closure, not a value (1.x API).
- genai's model-name routing means "custom" ids work for any family genai knows even
  when absent from OUR catalog — the custom row is honest, not a workaround.

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # 120 passed (was 104; +16)
cargo run -q -p epsilon-cli             # demo unchanged
cargo run -q -p epsilon-cli -- --list-models [provider]   # table w/ key status
Live wiring smoke (temp workspace):
  .epsilon/skills/test-skill/SKILL.md      → stderr: "skills: test-skill"
  .epsilon/extensions/demo (ext-fixture)   → stderr: "extension 'demo' v0.1.0 attached (1 tools)"
Saved-config precedence                    → stderr: "model: deepseek-chat (from saved config)"
```

## Known gaps remaining (owned by M12)

- Live per-provider listing (genai `all_model_names`), favorites/recents memory
- Full ADR-012: project layer, trust hash, ruleset/ext/MCP declarations, `config dump`
- Retry-with-durable-ledger, run-scoped cancellation, cost display, MCP instructions→
  CONTEXT tier + TTL probes, TUI card polish (markdown/themes), `--resume`
- rpc(stdio) + OpenAPI emission; Linux confinement verification (needs Unix host)
