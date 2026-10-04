# epsilon-contracts

## What this crate is

The shared vocabulary every other crate compiles against: messages, provider stream
events, agent events, session-log entries, tool descriptors, the command registry both front
ends read, and the traits that connect them. It owns types and trait signatures only — no
decisions, no filesystem, no I/O — and has zero internal dependencies, deliberately
([architecture §2](../../docs/design/epsilon-architecture.md)).

## How it connects

- **Depends on:** nothing in this workspace — only `serde`, `serde_json`, `async-trait`,
  `tokio-util`.
- **Depended on by:** every other crate: `epsilon-llm`, `epsilon-session`, `epsilon-core`,
  `epsilon-tools`, `epsilon-safety`, `epsilon-lsp`, `epsilon-ext`, `epsilon-mcp`,
  `epsilon-runtime` and all three front ends.

## Entry points

- `tool` — the `Tool` trait (`src/tool.rs:435`) every callable capability implements, plus
  `ToolDescriptor` (`:33`), `ExecutionMode` (`:24`), `ToolContext` (`:336`), and the seams
  `ToolHooks` (`:183`), `TaskHost` (`:286`), `GoalHost` (`:313`).
- `stream` — `StreamEvent` (`src/stream.rs:18`) and `StreamFolder` (`:67`), the one place
  that knows how deltas become blocks.
- `agent_event` — `AgentEvent` (`src/agent_event.rs:105`), the only output of the loop;
  every front end is a reducer over it. `TurnEndReason` (`:14`) and `PermissionDecision`
  (`:66`) live here too.
- `message` — `Message` (`src/message.rs:304`), `Usage` (`:120`), `StopReason` (`:93`).
- `session_entry` — the append-only JSONL vocabulary: `SessionEntry`
  (`src/session_entry.rs:99`), `SessionHeader` (`:44`), `EntryKind` (`:169`).
- `command` — what a `/word` means, for both front ends at once: `parse_command`
  (`src/command.rs:464`), `Command`, `SLASH_ITEMS`, `COMMAND_HEADS` and `ALIAS_HEADS`. It moved
  here from `epsilon-tui/src/picker.rs` at M290 because the desktop was reading it *through a
  terminal crate*; `picker` re-exports all five, so no call site changed.
- Smaller modules: `ids` (`new_id`, `src/ids.rs:14`), `content` (`ContentBlock`), `goal`
  (`GoalSnapshot`, `GoalView`), `queue_mode`, `turn` (`TurnHooks`), `prompt_template`,
  `diagnostics`, `session_state`.

## How to run and test it

No binary of its own — there is nothing to launch; it is compiled by its dependents. There
is no `tests/` directory, so the test run is the inline `#[cfg(test)]` units plus doc tests.

```sh
cargo check -p epsilon-contracts
cargo test -p epsilon-contracts
```

## Things worth knowing

- Every serde name here is a contract. `EntryKind`'s codec is lenient about the *tag* and
  strict about the *body*: an unknown `type` is preserved verbatim
  (`src/session_entry.rs:305`, `:344`), a known one with an unreadable body is refused.
  `KNOWN_ENTRY_KINDS` (`:152`) must list every variant, and a test fails when it does not.
- A provider stream ends with exactly one `StreamEvent::Done`, and its `StopReason` carries
  failure as data — providers never "throw" (`src/stream.rs:14`).
- `Tool::resource_key` should return a normalised absolute path: a planner that cannot tell
  `/a/b` from `/a/./b` serialises work that could have run in parallel
  (`src/tool.rs:449`). `ExecutionMode::Sequential` with no `resource_key` is a whole-batch
  barrier; with one, it orders only against calls naming the same resource (`:17`).
- `TurnEndReason::merge_sticky` keeps the worse reason, so `MaxTokens` survives a later
  clean step (`src/agent_event.rs:32`).
- `ToolDescriptor::basic_validate` checks shape only — that the arguments are a JSON object
  (`src/tool.rs:66`). Full schema validation belongs to the tools crate.