# M21 — Thinking fidelity sweep (DIRECTION-04): pi parity on the hottest path

Date: 2026-08-25. Trigger: the standing loop ("implement slice → gates green →
re-read references → close gaps"). `docs/plans/README.md` recommended DIRECTION-04
after 05 closed in M20: thinking/reasoning effort is pi's most-missed capability
on the hottest path, and Epsilon had the session-entry vocabulary
(`EntryKind::ThinkingLevelChange`) with NOTHING reading or writing it.

## The gap, verified in both trees

pi carries a shared `ThinkingLevel` ladder (`off..max`,
`agent/src/types.ts:300`) per session; each provider API maps it natively:

- Anthropic (`api/anthropic-messages.ts:1049-1073`): adaptive thinking +
  `output_config.effort` for Claude 4.5+; budget-based `thinking.enabled` for
  older models; explicit disable when off.
- OpenAI-compat (`api/openai-completions.ts:747-840`): plain
  `reasoning_effort` for gpt/o-series; nested `reasoning.effort` for
  OpenRouter; `thinking.type + clear_thinking` for z.ai GLM;
  `enable_thinking` for qwen families.
- Google (`api/google-generative-ai.ts:386-397`): `generationConfig.thinkingConfig`
  with `includeThoughts: true` + `thinkingLevel` enum.

Epsilon sent NONE of these. Thinking deltas streamed back and were rendered,
but we could never ASK a model to think harder — on models that bill reasoning
separately this is also a cost lever.

## What shipped

### contracts (`epsilon-contracts::message::ThinkingLevel`)
The seven-rung ladder as a Copy enum with `ALL`, `next()` (pi cycle semantics),
`as_str`, `parse`, `enabled`. Serde snake_case matches the JSONL entry docs.

### llm (`LlmContext.thinking_level` + wire mappings)
`LlmContext` gains `thinking_level`; every provider maps it at request-build:
- **anthropic.rs**: adaptive `{"type":"adaptive","display":"summarized"}` +
  `output_config.effort` (minimal/low→low, medium→medium, high→high,
  xhigh→xhigh; max omitted — no native value yet). `Off` sends nothing.
  ponytail note: budget-based thinking returns if a pre-adaptive Claude row
  ever enters the generated catalog (none exists today).
- **openai_compat.rs** `apply_thinking`: openrouter ids → nested
  `reasoning.effort`; deepseek-reasoner rows → `thinking:{enabled}` +
  `reasoning_effort`; glm/zai → `thinking{clear_thinking:false}` +
  `reasoning_effort`; kimi-thinking rows → `reasoning_effort`; gpt/o-series →
  `reasoning_effort`; unknown compat ids get the standard knob (loud failure
  beats silent misroute, same posture as pi's default branch). `Off` sends nothing.
- **google.rs**: `generationConfig.thinkingConfig{includeThoughts:true,
  thinkingLevel}` (MINIMAL..HIGH uppercase; xhigh/max clamp to HIGH).

Fixture tests pin all three shapes plus the Off-sends-nothing invariant.

### core (`Agent` session state)
`set_thinking_level`/`thinking_level` mirror `model_id`; the level rides every
run through the context build. Compaction summarizer stays hardwired `Off`
(ADR-007: summaries are isolated cheap calls).

### tui (pi interaction, terminal-reality keybind)
- `/think <level>` sets; bare `/think` reports current; unknown words are
  corrected inline (never sent to the model).
- Ctrl+T cycles off→minimal→…→max→off with an immediate notice. pi uses
  Shift+Tab; that combo is unreliable across terminals (and our conhost
  reality), so we take unclaimed Ctrl+T — same rationale as M17's keybind sweep.
- Help line + slash autocomplete updated.

## Test-caught bugs / gotchas

- My first draft used the notices CHANNEL for command replies; the two new
  key-path tests failed because channel messages only surface via the event
  loop (absent under `handle_key`). run_command replies push DIRECTLY into
  UiState — the convention was already documented at the fn; I hadn't read it.
  Lesson: match the sibling call sites before choosing a transport.
- `starts_with(("a","b"))` isn't a Pattern for &str tuples — use an array iter.
- The wire-fixture ToolDescriptor needed label/execution_mode fields; copy the
  struct literally from an existing fixture instead of reconstructing from memory.

## Verification

```
cargo fmt --all -- --check                → clean
cargo clippy --workspace --all-targets    → 0 warnings (both feature configs)
cargo test --workspace                    → all green (+2 TUI key-path, +3 wire fixtures)
cargo test -p epsilon-llm --features real → 33 green
cargo run -q -p epsilon-cli -- --demo     → completes
cargo check -p epsilon-cli --features real→ ok
```

## Known limits / next

- Level is session state only; persisting across restarts would want config
  plumbing like recent-models (recorded, not built — YAGNI until asked).
- No UI affordance showing the active level in the footer yet; notices cover it.
- Per-model clamping uses family prefixes, not models.dev `reasoning_options`
  (which our generator drops). If a future model diverges, extend the catalog
  generator to emit a thinking map rather than growing prefix tables.
