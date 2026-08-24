# M17 — pi-agent provider + TUI port

**Trigger.** User review: "the harness is still behind ./refs — model picker lists
obsolete models, slash suggestions can't be chosen with arrow keys, shortcuts are
missing. Port from pi-agent; enhance with other references; full credit." A later
directive made it explicit: port provider connectivity AND TUI/logic from pi,
enhance from opencode/DSH/Hermes, keep AGENTS.md/README/CHANGELOG/docs current.

## What landed

### 1. Provider layer: pi's direct-HTTP architecture (replaces genai)

pi talks to each provider's real REST API over plain HTTP; no router SDK sits in
between. Epsilon now does the same:

| File | Role | pi origin |
|---|---|---|
| `epsilon-llm/src/http.rs` | SSE line decoder + pooled client | `packages/ai/src/api/anthropic-messages.ts` SSE handling (`event:`/`data:` lines, blank-line frames, CRLF) |
| `epsilon-llm/src/providers/anthropic.rs` | `/v1/messages` wire | `api/anthropic-messages.ts`: system as cache-controlled text block, `x-api-key` + `anthropic-version: 2023-06-01`, `message_start`→usage seed, `text_delta`/`thinking_delta`/`input_json_delta`, `message_delta`→final usage + stop reason, tool results replayed as `tool_result` user turns |
| `epsilon-llm/src/providers/openai_compat.rs` | one wire for OpenAI-compatible providers | `api/openai-completions.ts`: `stream_options.include_usage`, indexed `tool_calls` deltas, `[DONE]`, `finish_reason` mapping |
| `epsilon-llm/src/providers/google.rs` | Gemini wire | `api/google-generative-ai.ts`: `:streamGenerateContent?alt=sse`, `x-goog-api-key`, `systemInstruction`, inline `functionCall` parts, functionResponse replay in user turns |
| `epsilon-llm/src/providers/mod.rs` | shared engine (`ProviderWire` trait + `HttpEventStream` accumulator + reqwest driver) | pi's Provider/Api split as a Rust object-safety boundary |

Design decisions:

- **Routing is catalog data.** Each catalog row carries a `wire: WireApi` column
  (pi's `model.api`). Nothing string-matches model families — the M3 anti-pattern
  rule now extends to transport selection.
- **Chunk-boundary safety**: bytes accumulate in a line buffer; only complete SSE
  lines decode. Multi-event frames queue instead of dropping.
- **Never-fail contract kept**: every pre-stream failure (unknown model, missing
  env key, HTTP error, connect failure) is a `Done { stop_reason: Error }` event;
  mid-stream read errors terminate honestly.
- **genai removed** from workspace deps; `reqwest =0.13.4` pinned exact (already
  in-tree via rmcp — zero new transitive crates).

Why genai had to go (user observation, confirmed by source reading): its
`AdapterKind::from_model` silently routed bare `kimi-*`/`glm-*`/`llama-*` ids to
its OLLAMA adapter; it invented its own env-var spelling (`OPEN_ROUTER_API_KEY`);
it carried no model metadata (which forced the models.dev dependency anyway).
Direct HTTP removes all three failure classes.

### 2. Catalog v2 — live models.dev regeneration

Fetched <https://models.dev/api.json> live (195 providers) and curated
tool-calling chat models per supported provider into `epsilon-llm/src/catalog.rs`:
current generations (Claude Opus/Sonnet 4.5–5, GPT-5.2–5.4 family, DeepSeek V4,
Gemini 2.5–3.6, Grok 4.3–4.6, Kimi K2.x/K3, GLM-4.7/5, Groq Llama+GPT-OSS),
provider-native ids, REAL published prices (models.dev rates ×1e6 micro-USD).

The footer cost line is therefore an estimate against public list prices, not a
billing feed — documented in the module header. Tests pin: unique ids,
wire-auth agreement (`wire_auth_matches_row_auth`), price sanity (real rates for
cloud, zero for local), current-generation guards (`catalog_is_current_generation`
asserts e.g. `claude-opus-5` present and `gpt-4o` absent).

Live listing (`real` builds) is now a plain authenticated GET `/models` per
provider — the pi/opencode convention — replacing genai's resolver.

### 3. TUI: slash autocomplete rebuilt on pi's pattern

Old behavior: Tab completed the FIRST candidate; arrows did nothing while the
popup showed; popup state was derived at draw time only.

New (ported semantics, pi `autocomplete.ts` + `components/editor.ts` +
opencode `prompt.autocomplete.*` keybinds):

- `picker::SlashItem` registry with descriptions (pi `AutocompleteItem` shape).
- Popup state lives in App (`slash_open`, `slash_index`) and TRACKS THE BUFFER via
  `sync_slash_state()` on insert/backspace/kill/undo — never lingers.
- **Up/Down move the highlight** when open (history recall suspended); Enter
  applies the highlighted row THEN falls through to submit (pi's
  applyCompletion → submit fall-through: "/clear" runs immediately); Tab applies;
  Esc dismisses before quit/abort.
- Renderer highlights the selected row REVERSED with dim description text
  (pi SelectList look).

### 4. Keybind parity sweep (opencode `config/keybind.ts` + pi `keybindings.ts`)

Ctrl+A/E line home/end · Ctrl+B/F char left/right · Alt+B/F + Alt+←/→ word jumps ·
Alt+Backspace word kill · Ctrl+D forward delete · Ctrl+K kill-to-line-end (feeds
the kill ring) · Ctrl+. redo · F2 cycles recent models. All multi-line aware
(`editor_ops::line_home/line_end/kill_to_line_end`). Ctrl+J newline, Ctrl+W/U/Y,
Ctrl+M picker, Ctrl+P palette unchanged.

### 5. Redo rail

`undo.rs`: `pop()` became `undo(current)` / `redo(current)` — the LIVE state being
left is recorded on the rail (first draft stored stale pre-mutation snapshots and
redo restored nothing; caught by the key-path test, fixed before landing). Fresh
push after undo clears the rail (branch divergence).

## Test-caught bugs (each cost a cycle)

1. **Redo rail semantics**: storing the undone snapshot instead of the live state
   meant Ctrl+. was a no-op that corrupted toggling. Test `ctrl_dot_redoes_what_
   ctrl_z_undid` failed; API changed to pass `current` explicitly.
2. **Enter interception**: first implementation swallowed Enter inside the popup
   ("/models" could never RUN). The old test `slash_models_opens_picker_via_real_
   key_path` caught it — fall-through restored, matching pi exactly.
3. **SSE fixture bug**: decoder test asserted `[DONE]` produces NO frame on its
   blank line — wrong, it completes like any event. Fixture fixed (pitfall #9).

## Attribution (full credit, per project rule)

- pi-0.84.2 (`references/pi-0.84.2`): provider architecture, all three wire
  adapters' request/response mapping, SSE conventions, autocomplete interaction
  model, undo-stack base. MIT-licensed project by Mario Zechner (badlogic) —
  patterns ported, not code copied (Rust reimplementation, semantics cited).
- opencode-1.18.21 (`references/opencode-1.18.21`): keybind default table
  (Ctrl+A/E/B/F/D/K, Alt+B/F/D, redo binding), two-stage picker precedent,
  stash pattern (M16c).
- models.dev registry: catalog data + prices (fetched 2026-08).

## Verification output

```
cargo fmt --all                          # clean
cargo clippy --workspace --all-targets   # 0 warnings
cargo test -p epsilon-tui                # 79 passed
cargo test -p epsilon-llm --features real # 24 passed
cargo test --workspace                   # all green
cargo run -q -p epsilon-cli -- --demo    # completes (6 messages, 300/150 tokens)
cargo check -p epsilon-cli --features real # compiles
```

## Known limits / next

- Anthropic thinking signatures are accepted but not round-tripped (no extended
  thinking replay yet) — same posture as genai era.
- Google adapter maps `thought:true` parts to text today; dedicated thinking
  channel is a follow-up.
- Catalog refresh is manual (regenerate script pending); models.dev snapshot date
  is stamped in the module header.
