# Epsilon architecture

For contributors locating runtime responsibilities in the audited working tree.
The [audit](../../audit/README.md) records verified defects; the
[ADR log](epsilon-decisions.md) records design intent. Accepted intent is not proof
of implemented behavior. The pi port remains incomplete.

## Workspace and dependency graph

The existing Cargo workspace already provides conventional `crates/<name>/src/`
and per-crate `tests/` directories. Unit tests also live beside their modules.
There is no need for another workspace layer or empty benchmark/example directories.

Direct internal production dependencies, as declared in crate manifests:

| Crate | Internal dependencies | Responsibility |
|---|---|---|
| `epsilon-contracts` | none | Shared types/traits; currently also validation and stream folding |
| `epsilon-llm` | contracts | Provider protocols, routing, faux provider, catalog, credentials, retry/fallback |
| `epsilon-session` | contracts | JSONL storage, branch projection, indexing, repair and leases |
| `epsilon-lsp` | contracts | Installed language-server discovery, clients and diagnostics |
| `epsilon-ext` | contracts | Process extension protocol and host |
| `epsilon-core` | contracts, llm, session | Agent state/loop, scheduling, context, compaction and task host |
| `epsilon-safety` | contracts, core | Permission rules, approvals, environment filtering and sandbox backends |
| `epsilon-tools` | contracts, core, safety, lsp | Tool implementations, file observations, mutation exclusion and checkpoints |
| `epsilon-mcp` | contracts, core | MCP connections and tool adaptation |
| `epsilon-tui` | contracts, core, llm, safety, session | Terminal application, editor, dialogs and transcript |
| `epsilon-server` | contracts, core, llm, safety, tools | HTTP/SSE frontend |
| `epsilon-cli` | all other workspace crates | Arguments, configuration, runtime composition and dispatch |

Cargo checks declared dependencies and rejects cycles; it does not enforce a
separate allowed-edge policy. Libraries must not depend on CLI. The intended
vocabulary-only contracts boundary has existing drift:
`crates/epsilon-contracts/src/stream.rs:62–134` implements stateful stream folding
and provider collision handling. Do not treat that as an approved policy expansion.

## Composition and execution

1. `crates/epsilon-cli/src/main.rs` parses modes/options. `runtime.rs` supplies
   configuration and provider/resource composition. CLI attaches tools, a permission
   gate and a durable store to `epsilon_core::Agent`.
2. `crates/epsilon-core/src/lib.rs:900–934` admits a run; `:1037–1261` alternates
   provider turns and tool batches with steering and follow-up queues. Core rejects
   overlapping runs, but frontend acknowledgments do not yet share atomic admission.
3. `crates/epsilon-core/src/exec.rs` validates and authorizes invocations, plans
   bounded resource-conflict groups and executes tools. Results are committed in
   model order; sequential tools force serial batch execution.
4. `crates/epsilon-llm/src/providers/` owns native protocol adapters. Shared transport
   lives in `providers/mod.rs`; `retry_stream.rs` and `fallback.rs` compose recovery.
5. `AgentEvent` broadcasts feed frontends and extension forwarding. This is not a
   durable replay protocol; RPC currently skips lagged broadcasts.

Pi's two-level loop is the reference; bounded conflict-aware scheduling is an
intentional difference. M85 aligns the follow-up default with pi's one-at-a-time
mode; explicit all-at-once remains available. Tool hooks exist, but configurable next-turn/stop
callbacks are not established, despite ADR-002's commitment. The
[port matrix](../../audit/03-port-plan.md) distinguishes mechanisms from compatibility.

## Tools and execution boundaries

Use the actual API declarations in `crates/epsilon-contracts/src/tool.rs`, not an
approximate signature copied into documentation. `epsilon-tools/src/lib.rs`
registers built-ins and optional services. File/search tools use native host calls;
BashTool receives the subprocess sandbox. MCP and process extensions are separate
adapters, not evidence that arbitrary plugin execution is confined by BashTool.

- `epsilon-safety/src/engine.rs:82–97`: matching denies take precedence; otherwise
  the later matching allow/ask rule determines the decision.
- Path identity remains lexical (F09). Core authorizes the original capability and
  any derived external-path subject. Plan mode retains project rules and denies
  native write/edit/bash/shell; it does not classify arbitrary plugin side effects.
  [Permissions](../user/permissions.md) owns the current policy description.
- `epsilon-safety/src/sandbox.rs`: Windows selects Job Objects; other platforms
  probe `bwrap` or report no available backend. Landlock and Seatbelt are not
  implemented backends in this tree.
- Windows Job Objects constrain resources/processes, not filesystem or network
  access. Bubblewrap mounts host root read-only and unshares namespaces; read-only
  does not mean unreadable. Default shell execution refuses without a backend.
- Process waits, cancellation and final pipe drains have inconsistent total-deadline
  ownership (F11). Display caps apply after unlimited parent output collection (F12).

`epsilon-tools/src/mutation.rs` serializes writers by path. `read_log.rs` scopes
observations to each agent's opaque lifetime token and invalidates sibling knowledge
after writes. Restore renews the token; retired observers are reclaimed. F02 repaired
shared knowledge, not resolved-object identity or race-resistant containment (F09).

## Persistence, context and recovery

`epsilon-session/src/store.rs` owns append/read/index/navigation and active-branch
model projection. Compaction appends summaries rather than deleting old messages.
That design does not establish correct persistence:

- Earlier summaries accumulate in subsequent compacted projections (F04).
- Core updates memory independently and discards durable mutation errors
  (`epsilon-core/src/lib.rs:743–777,1497–1512`, F07).
- Some complete unsupported final entries enter torn-tail truncation (F08).

Leases live in `epsilon-session/src/lease.rs`. The TUI's `/fork` copies a session file
without switching to it; `/tree` shows the active branch. No Rust `BranchSummary`
producer was found. Neither the JSONL shape nor familiar commands establish pi
session-file compatibility.

`epsilon-core/src/prompt.rs` assembles stable/context/volatile sections;
`compact.rs` owns context selection and summary construction. Deterministic assembly
supports cache reuse but cannot guarantee provider cache hits. Summary acceptance
and cancellation require hardening (F06), as does persistence of local cancellation
salvage (F17). Byte-identical replay of every provider request is not established.

`epsilon-tools/src/checkpoint.rs` records preimages before edits. Skipped captures
are not safely distinguished from creation at restore, ledger errors are swallowed,
and later user changes lack conflict checks (F15). Capture is implemented; safe
user undo is not established. Do not replace it with a second snapshot subsystem
before its existing contracts work.

## Frontends and integrations

| Surface | Implementation and limits |
|---|---|
| TUI | `epsilon-tui/src/app.rs` orchestrates editor/picker/markdown/state modules. Ownership is concentrated; split stable responsibilities only after characterization and baseline repair. |
| HTTP | `epsilon-server/src/lib.rs` owns routing, SSE and request guards. Admission and dropped-event recovery remain open. |
| RPC | `epsilon-cli/src/rpc_mode.rs`: prompt, steer, followup, approvals, approve, ping. Not pi's envelope or full cancellation/state/session surface. |
| Extensions | `epsilon-ext/src/manifest.rs` and `host.rs` define the implemented process protocol, not pi's complete extension SDK. |
| MCP | `epsilon-mcp` adapts stdio servers through rmcp and registers tools. |
| LSP | `epsilon-lsp` discovers user-installed servers and appends bounded-count error reports. Monotonic versions, total deadlines, cancellation-safe framing and freshness remain open (F14). |

Provider fixtures and compilation do not establish every catalog model or OAuth flow.
Shared SSE buffering/EOF behavior and refresh/fallback preparation have open defects
(F03/F16). User-facing configuration and commands belong in the
[user guides](../user/README.md), not duplicate API examples here.

## Maintenance and correction record

[AGENTS.md](../../AGENTS.md) owns contributor rules and verification commands.
[Internal inconsistencies](../../audit/07-inconsistencies.md) records competing
ownership models; [the roadmap](../../audit/04-roadmap.md) orders their repair.

The previous guide mixed proposals with implementation: Landlock, Seatbelt,
tree-sitter shell analysis, complete callback parity, branch summaries, exact replay,
broad extension features and universal cleanup were stated as existing. Those
claims and approximate Rust API snippets are removed. Design rationale remains in
the ADR log and Git history; removing an unsupported claim is not deleting a feature
or silently changing the port target. This guide makes no new upstream version,
performance, platform-certification or license-provenance claims.
