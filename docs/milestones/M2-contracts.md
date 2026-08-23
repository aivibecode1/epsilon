# M2 — epsilon-contracts (the stud standard)

Date: 2026-08-23 · Status: complete

## What was built

Seven modules of pure types + one trait, ported semantically from pi:

| Module | Contents | Pi origin |
|---|---|---|
| `ids.rs` | `new_id(prefix)` monotonic sortable ids, id aliases | pi's uuid usage, dependency-free variant |
| `content.rs` | `ContentBlock` (Text/Thinking/Image/ToolCall), `UserContent`, `ImageRef` | content unions + first-class thinking |
| `message.rs` | `Message` enum, `UserMessage`, `AssistantMessage`, `ToolResultMessage`, `CustomMessage`, `StopReason`, `Usage`+micro-USD cost | `types.ts` core |
| `stream.rs` | `StreamEvent` deltas + `StreamFolder` (single place folding rules live) | `AssistantMessageEvent` |
| `agent_event.rs` | `AgentEvent` lifecycle taxonomy, `TurnEndReason` sticky merge, permission request/decision | AgentEvent + DSH turn endings |
| `session_entry.rs` | `SessionEntry`/`EntryKind` incl. Compaction/BranchSummary/TurnEnd + api hash field | `harness/session/types.ts` |
| `tool.rs` | `Tool` trait (async_trait, object-safe), descriptor/context/partial/output/failure types | `AgentTool` |

15 unit tests; serde roundtrips for every wire type.

## Key decisions made while coding

1. **Rust has no TS declaration merging.** pi's extensible `CustomAgentMessages` became a
   flattened `Message::Custom { custom_type, payload }`. Tradeoff: no compile-time typing for
   custom payloads; gained: zero-macro extensibility, stable wire format for the future
   extension protocol.
2. **Stream folding rule clarified the hard way** (test failure forced it): the `Done`
   message is *authoritative* (pi semantics); deltas exist only to feed live UI partials.
   First implementation tried to assemble from deltas and merge at Done — ambiguous when
   Done disagrees with accumulated deltas. The test caught the ambiguity before any consumer
   existed, which is exactly why contracts get written first.
3. **Truncated tool-call arguments are detectable by construction**: arguments arrive as JSON
   strings and are parsed only at Done; a MaxTokens-truncated string parses to `Null`, never
   to plausible-looking garbage. This is the contracts-level foundation for pi's
   fail-all-tool-calls-on-truncation rule.
4. **Costs in integer micro-USD**, not f64 dollars — long sessions must not accumulate drift
   (borrowed from ZSeven-W/agent-rs).
5. **Schema as raw JSON Value in the descriptor**, not schemars types — keeps contracts
   dependency-light; built-ins derive schemas via schemars in their own crate later.

## Gotchas hit (all Rust-learning-relevant)

- serde derive requires an explicit feature (`features = ["derive"]`) — not default.
- Match arms that destructure owned fields partially-move the scrutinee even if you only
  borrow afterwards; fix is `ref` patterns or explicit early returns. Hit this three times in
  `StreamFolder::apply`; the compiler walked us through each.
- Exact-pin discipline paid off immediately: `serde_json = "1.1.1"` doesn't exist; cargo's
  error listed real candidates (`1.0.151`) and the pin was corrected in seconds.

## Verification

```
cargo fmt --all -- --check        # clean
cargo clippy --workspace -D warnings level: zero warnings
cargo test -p epsilon-contracts   # 15 passed
```

## Next

M3 fills `epsilon-llm`: StreamFn trait + FauxProvider + genai adapter behind feature flag.
